//! Post-scan reports (cleanup candidates, inactive large files) written as JSON in the same
//! shape the Python CLI used, so the frontend readers stay unchanged.

use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BROAD_CONTAINERS: [&str; 7] = ["Downloads", "Desktop", "Documents", ".cache", "Library/Caches", "Library/Logs", "Library/Developer/Xcode/DerivedData"];

fn inside(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

fn meta_string(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

/// Rebuildable first, then review, largest first; no overlapping selections; broad containers skipped.
/// Classification is applied live from the current rules, so this works on older scans too.
pub fn write_candidates(db: &Path, out: &Path, home: &Path, limit: usize) -> Result<usize, String> {
    use crate::classify::{classify, RULES_VERSION};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let root = meta_string(&conn, "root").unwrap_or_default();
    let home_str = home.to_string_lossy().to_string();
    let containers: Vec<String> = BROAD_CONTAINERS.iter().map(|c| format!("{}/{}", home_str, c)).collect();

    // Anything below 1 MB is noise for a suggestions list; the 30k cap keeps huge scans quick.
    let mut stmt = conn
        .prepare("SELECT path, kind, total FROM entries WHERE total >= 1048576 AND error IS NULL ORDER BY total DESC LIMIT 30000")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
        .map_err(|e| e.to_string())?;

    let mut scored: Vec<(u8, String, String, i64, crate::classify::Classification)> = Vec::new();
    for (path, kind, total) in rows.flatten() {
        if path == root || containers.iter().any(|c| c == &path) {
            continue;
        }
        let c = classify(&path, &home_str);
        let rank = match c.category {
            "rebuildable" => 0,
            "review" if c.action != "report_only" => 1,
            _ => continue,
        };
        scored.push((rank, path, kind, total, c));
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.3.cmp(&a.3)));

    // Each base category gets its own quota so review items are not crowded out by caches.
    let mut selected: Vec<serde_json::Value> = Vec::new();
    let mut chosen: Vec<String> = Vec::new();
    let mut per_cat: [usize; 2] = [0, 0];
    for (rank, path, kind, total, c) in scored {
        if per_cat[rank as usize] >= limit {
            continue;
        }
        if chosen.iter().any(|x| inside(&path, x) || inside(x, &path)) {
            continue;
        }
        per_cat[rank as usize] += 1;
        chosen.push(path.clone());
        selected.push(json!({
            "path": path, "kind": kind, "total": total,
            "category": c.category, "reason": c.reason, "tag": c.tag, "action": c.action, "cost": c.cost, "tool": c.tool,
        }));
        if selected.len() >= limit * 2 {
            break;
        }
    }
    let n = selected.len();
    let doc = json!({ "schema": 2, "rules": RULES_VERSION, "scan": db.to_string_lossy(), "root": root, "items": selected });
    fs::write(out, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(n)
}

fn iso(secs: i64) -> Option<String> {
    if secs <= 0 {
        return None;
    }
    let dt = chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH + Duration::from_secs(secs as u64));
    Some(dt.to_rfc3339())
}

/// Parse `mdls -raw -name kMDItemLastUsedDate` output ("2026-05-18 17:53:50 +0000" or "(null)").
pub(crate) fn parse_mdls_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "(null)" {
        return None;
    }
    chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z").ok().map(|d| d.timestamp())
}

async fn spotlight_last_used(paths: Vec<String>, budget: Duration) -> std::collections::HashMap<String, Option<i64>> {
    use tokio::process::Command;
    let mut out = std::collections::HashMap::new();
    let deadline = tokio::time::Instant::now() + budget;
    let mut set = tokio::task::JoinSet::new();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    for p in paths {
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await.ok();
            let res = tokio::time::timeout(
                Duration::from_secs(2),
                Command::new("/usr/bin/mdls").arg("-raw").arg("-name").arg("kMDItemLastUsedDate").arg(&p).output(),
            )
            .await;
            let val = match res {
                Ok(Ok(o)) if o.status.success() => Some(parse_mdls_date(&String::from_utf8_lossy(&o.stdout))),
                _ => None,
            };
            (p, val)
        });
    }
    while let Ok(Some(Ok((p, v)))) = tokio::time::timeout_at(deadline, set.join_next()).await {
        // `None` means the lookup itself failed or timed out; `Some(None)` means Spotlight has no date.
        out.insert(p, v.flatten());
    }
    set.abort_all();
    out
}

/// Large files whose newest activity evidence is older than `days`.
pub async fn write_unused(db: PathBuf, out: PathBuf, days: u64, min_size: u64, limit: usize) -> Result<usize, String> {
    let (root, rows) = tokio::task::spawn_blocking({
        let db = db.clone();
        move || -> Result<(String, Vec<(String, i64, i64, i64, String, String)>), String> {
            let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
            let root = meta_string(&conn, "root").unwrap_or_default();
            let mut stmt = conn
                .prepare("SELECT path, size, allocated, mtime_ns, category, reason FROM entries WHERE kind='file' AND allocated >= ?1 AND error IS NULL ORDER BY allocated DESC LIMIT 3000")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([min_size as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?)))
                .map_err(|e| e.to_string())?
                .flatten()
                .collect::<Vec<_>>();
            Ok((root, rows))
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let cutoff = now - (days as i64) * 86400;

    // Filesystem evidence first (cheap), then Spotlight for the survivors within a time budget.
    struct Cand {
        path: String,
        size: i64,
        allocated: i64,
        atime: i64,
        mtime: i64,
        ctime: i64,
        birth: i64,
        category: String,
        reason: String,
    }
    let mut cands: Vec<Cand> = Vec::new();
    let home_live = crate::home_dir().to_string_lossy().to_string();
    for (path, size, allocated, mtime_ns, _category, _reason) in rows {
        let live = crate::classify::classify(&path, &home_live);
        let (category, reason) = (live.category.to_string(), live.reason);
        let Ok(m) = fs::symlink_metadata(&path) else { continue };
        if !m.is_file() || m.size() as i64 != size || m.mtime() * 1_000_000_000 + m.mtime_nsec() != mtime_ns {
            continue; // changed since the scan
        }
        let birth = m.created().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        let newest = m.atime().max(m.mtime()).max(m.ctime()).max(birth);
        if newest > cutoff {
            continue;
        }
        cands.push(Cand { path, size, allocated, atime: m.atime(), mtime: m.mtime(), ctime: m.ctime(), birth, category, reason });
        if cands.len() >= limit * 3 {
            break;
        }
    }

    let spot = spotlight_last_used(cands.iter().map(|c| c.path.clone()).collect(), Duration::from_secs(20)).await;

    let mut items = Vec::new();
    for c in cands {
        let looked_up = spot.contains_key(&c.path);
        let last_used = spot.get(&c.path).cloned().flatten();
        if let Some(lu) = last_used {
            if lu > cutoff {
                continue;
            }
        }
        let newest = c.atime.max(c.mtime).max(c.ctime).max(c.birth).max(last_used.unwrap_or(0));
        let days_since = ((now - newest) / 86400).max(0);
        let evidence = if last_used.is_some() { "spotlight_last_used" } else if looked_up { "filesystem_dates_only" } else { "filesystem_dates_only_spotlight_skipped" };
        items.push(json!({
            "path": c.path,
            "allocated_bytes_estimate": c.allocated,
            "logical_bytes": c.size,
            "category": c.category,
            "policy_reason": c.reason,
            "last_used": last_used.and_then(iso),
            "last_accessed": iso(c.atime),
            "modified": iso(c.mtime),
            "created": iso(c.birth),
            "latest_activity_evidence": iso(newest),
            "days_since_activity_evidence": days_since,
            "evidence": evidence,
            "usage_note": if last_used.is_some() { "Spotlight last-used date available." } else { "No indexed last-used date available." },
            "decision": "review",
            "next_step": "Review the full path; reveal in Finder for manual removal, or collect it in the app if eligible.",
        }));
        if items.len() >= limit {
            break;
        }
    }
    let n = items.len();
    let doc = json!({
        "schema": 1,
        "scan": db.to_string_lossy(),
        "root": root,
        "days": days,
        "min_size_bytes": min_size,
        "kind": "files",
        "items": items,
        "notice": "Review candidates only. Last access may be stale, disabled, or updated by background tools; modification/creation are not usage. Spotlight can be absent or incomplete. No deletion is authorized by age.",
    });
    fs::write(&out, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(n)
}
