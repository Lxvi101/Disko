//! Everything with an age: apps by last use, projects by last git activity, big files and downloads by
//! last open, simulators by last boot, toolchains by install date, device backups by backup date.

use crate::classify::classify;
use crate::findings::{has_ancestor, in_user_space, junk};
use rayon::prelude::*;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

#[derive(Serialize, Clone, Debug)]
pub struct OldItem {
    pub path: String,
    pub name: String,
    /// Timeline lane: app, project, file, download, developer, backup.
    pub lane: &'static str,
    /// Plain-language kind within the lane, e.g. "Simulator" or "Node.js".
    pub label: String,
    pub total: u64,
    /// Newest evidence of use, seconds since the epoch. `None` when macOS keeps no record.
    pub date: Option<i64>,
    /// What `date` means: opened, worked on, modified, added, booted, installed, backed up.
    pub date_kind: &'static str,
    pub detail: String,
    pub category: String,
    pub action: String,
    pub tool: Option<String>,
    /// For projects: generated folders inside (path, size) that can go without touching the source.
    pub junk: Vec<(String, u64)>,
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn mtime(path: &Path) -> Option<i64> {
    fs::metadata(path).ok().map(|m| m.mtime()).filter(|t| *t > 0)
}

fn db_total(conn: &Connection, path: &str) -> Option<u64> {
    conn.query_row("SELECT total FROM entries WHERE path = ?1", [path], |r| r.get::<_, i64>(0)).ok().map(|t| t.max(0) as u64)
}

/// Spotlight attributes for many paths in a few `mdls` calls. Missing paths make mdls skip output,
/// so paths must exist; a chunk whose output does not line up is discarded rather than misread.
fn spotlight(paths: &[String], attrs: &[&str]) -> HashMap<String, Vec<Option<i64>>> {
    let chunks: Vec<&[String]> = paths.chunks(250).collect();
    chunks
        .par_iter()
        .flat_map_iter(|chunk| {
            let mut cmd = Command::new("/usr/bin/mdls");
            cmd.args(["-raw", "-nullMarker", ""]);
            for a in attrs {
                cmd.args(["-name", a]);
            }
            let out = cmd.args(chunk.iter()).output().ok();
            let fields: Vec<String> = out
                .map(|o| o.stdout.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).to_string()).collect())
                .unwrap_or_default();
            let mut res = Vec::new();
            // Output ends with a trailing separator on some macOS versions.
            let n = chunk.len() * attrs.len();
            if fields.len() == n || (fields.len() == n + 1 && fields[n].is_empty()) {
                for (i, p) in chunk.iter().enumerate() {
                    let vals = (0..attrs.len()).map(|j| crate::reports::parse_mdls_date(&fields[i * attrs.len() + j])).collect();
                    res.push((p.clone(), vals));
                }
            }
            res
        })
        .collect()
}

fn size_cache_path() -> std::path::PathBuf {
    crate::reports_dir().join("app-sizes.json")
}

/// Bundle sizes by path, stamped with the bundle's modification time. Kept on disk because
/// measuring every app takes seconds and bundles only change when they update.
fn size_cache() -> &'static Mutex<HashMap<String, (i64, u64)>> {
    static C: OnceLock<Mutex<HashMap<String, (i64, u64)>>> = OnceLock::new();
    C.get_or_init(|| {
        let saved = fs::read_to_string(size_cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Mutex::new(saved)
    })
}

/// On-disk size of a bundle, from the scan when it covered the path, otherwise measured once per version.
fn bundle_size(conn: Option<&Connection>, path: &str) -> u64 {
    if let Some(t) = conn.and_then(|c| db_total(c, path)) {
        return t;
    }
    let stamp = mtime(Path::new(path)).unwrap_or(0);
    if let Some((s, size)) = size_cache().lock().ok().and_then(|c| c.get(path).copied()) {
        if s == stamp {
            return size;
        }
    }
    let size = crate::xcode::allocated_size(Path::new(path)).0;
    if let Ok(mut c) = size_cache().lock() {
        c.insert(path.to_string(), (stamp, size));
    }
    size
}

/// Newest trace an app leaves in the home Library while it runs: preferences, saved window state,
/// its container and caches. Used only when Spotlight has no last-used date.
fn footprint(home: &str, bundle_id: &str) -> Option<i64> {
    ["Library/Preferences/{}.plist", "Library/Saved Application State/{}.savedState", "Library/Containers/{}", "Library/Caches/{}", "Library/HTTPStorages/{}", "Library/Application Support/{}"]
        .iter()
        .filter_map(|t| mtime(Path::new(&format!("{}/{}", home, t.replace("{}", bundle_id)))))
        .max()
}

fn plist_string(path: &Path, key: &str) -> Option<String> {
    plist::Value::from_file(path).ok()?.as_dictionary()?.get(key)?.as_string().map(str::to_string)
}

fn apps(home: &str) -> Vec<OldItem> {
    let mut paths = Vec::new();
    for dir in ["/Applications".to_string(), format!("{}/Applications", home)] {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if name.ends_with(".app") {
                paths.push(p);
            } else if p.is_dir() {
                // Vendor folders such as "Adobe Photoshop 2026" hold the app one level down.
                if let Ok(inner) = fs::read_dir(&p) {
                    paths.extend(inner.flatten().map(|i| i.path()).filter(|i| i.extension().is_some_and(|x| x == "app")));
                }
            }
        }
    }
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|p| fs::symlink_metadata(p).map(|m| !m.file_type().is_symlink() && !crate::is_sip_restricted(&m)).unwrap_or(false))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let used = spotlight(&paths, &["kMDItemLastUsedDate"]);
    let sizes: Vec<u64> = {
        // Measure in parallel; each worker needs its own read-only connection.
        let db = crate::db_path();
        paths
            .par_iter()
            .map_init(|| Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok(), |c, p| bundle_size(c.as_ref(), p))
            .collect()
    };
    if let Ok(c) = size_cache().lock() {
        let _ = fs::write(size_cache_path(), serde_json::to_string(&*c).unwrap_or_default());
    }
    paths
        .into_iter()
        .zip(sizes)
        .filter_map(|(path, total)| {
            let c = classify(&path, home);
            if c.category == "protected" {
                return None;
            }
            let info = Path::new(&path).join("Contents/Info.plist");
            let version = plist_string(&info, "CFBundleShortVersionString");
            let (date, date_kind) = match used.get(&path).and_then(|v| v[0]) {
                Some(d) => (Some(d), "opened"),
                None => (plist_string(&info, "CFBundleIdentifier").and_then(|id| footprint(home, &id)), "last seen"),
            };
            Some(OldItem {
                name: name_of(&path).trim_end_matches(".app").to_string(),
                lane: "app",
                label: "App".into(),
                total,
                date,
                date_kind,
                detail: version.map(|v| format!("Version {}", v)).unwrap_or_default(),
                category: c.category.into(),
                action: c.action.into(),
                tool: c.tool,
                junk: vec![],
                path,
            })
        })
        .collect()
}

fn projects(conn: &Connection, home: &str, junk_items: &[(String, u64)]) -> Vec<OldItem> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT p.path, p.total FROM entries e JOIN entries p ON p.id = e.parent WHERE e.kind = 'directory' AND e.path LIKE '%/.git'",
    ) else {
        return vec![];
    };
    let roots: Vec<(String, u64)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as u64)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    // Repositories vendored inside dependency caches or build trees are not projects.
    let roots: Vec<(String, u64)> = roots
        .into_iter()
        .filter(|(p, _)| {
            let c = classify(p, home);
            ["user-data", "user-files", "downloads"].contains(&c.tag) && in_user_space(&format!("{}/x", p), home)
        })
        .collect();
    let set: HashSet<String> = roots.iter().map(|r| r.0.clone()).collect();
    roots
        .into_iter()
        .filter(|(p, _)| !has_ancestor(p, &set))
        .filter_map(|(path, total)| {
            let git = Path::new(&path).join(".git");
            // The index changes on commits, checkouts and every `git status`, so it tracks when the project was last open.
            let date = ["index", "HEAD", "logs/HEAD"].iter().filter_map(|f| mtime(&git.join(f))).max()?;
            let prefix = format!("{}/", path);
            let inside: Vec<&(String, u64)> = junk_items.iter().filter(|(p, _)| p.starts_with(&prefix)).collect();
            let c = classify(&path, home);
            Some(OldItem {
                name: name_of(&path),
                lane: "project",
                label: "Project".into(),
                total,
                date: Some(date),
                date_kind: "worked on",
                detail: String::new(),
                category: c.category.into(),
                action: c.action.into(),
                tool: None,
                junk: inside.into_iter().cloned().collect(),
                path,
            })
        })
        .collect()
}

const PERSONAL_TAGS: &[&str] = &["user-files", "downloads", "old-installer", "user-data"];

fn files_and_downloads(conn: &Connection, home: &str) -> Vec<OldItem> {
    let mut rows: Vec<(String, String, u64, &'static str)> = Vec::new();
    // Everything directly in Downloads and on the Desktop, files and folders alike.
    for dir in ["Downloads", "Desktop"] {
        let q = "SELECT e.path, e.kind, e.total FROM entries e JOIN entries p ON p.id = e.parent WHERE p.path = ?1 AND e.total >= 5242880";
        if let Ok(mut st) = conn.prepare(q) {
            if let Ok(r) = st.query_map([format!("{}/{}", home, dir)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?.max(0) as u64))) {
                rows.extend(r.flatten().map(|(p, k, t)| (p, k, t, "download")));
            }
        }
    }
    // Large files anywhere else people keep their own things.
    if let Ok(mut st) = conn.prepare("SELECT path, kind, allocated FROM entries WHERE kind = 'file' AND allocated >= 52428800 AND error IS NULL ORDER BY allocated DESC LIMIT 3000") {
        if let Ok(r) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?.max(0) as u64))) {
            rows.extend(r.flatten().map(|(p, k, t)| (p, k, t, "file")));
        }
    }
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(p, ..)| {
            let c = classify(p, home);
            PERSONAL_TAGS.contains(&c.tag) && c.action == "quarantine" && in_user_space(p, home) && Path::new(p).exists()
        })
        .collect();
    let paths: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let spot = spotlight(&paths, &["kMDItemLastUsedDate", "kMDItemDateAdded"]);
    rows.into_iter()
        .map(|(path, kind, total, lane)| {
            let s = spot.get(&path);
            let used = s.and_then(|v| v[0]);
            let added = s.and_then(|v| v[1]);
            let modified = mtime(Path::new(&path));
            let (date, date_kind) = match (used, added, modified) {
                (Some(u), a, _) if u >= a.unwrap_or(0) => (Some(u), "opened"),
                (_, Some(a), _) if lane == "download" => (Some(a), "added"),
                (u, a, m) => (u.max(a).max(m), "modified"),
            };
            let c = classify(&path, home);
            let ext = if kind == "directory" { "Folder".to_string() } else { path.rsplit_once('.').map(|(_, e)| e.to_ascii_uppercase()).filter(|e| e.len() <= 5).unwrap_or_else(|| "File".into()) };
            OldItem {
                name: name_of(&path),
                lane,
                label: if c.tag == "old-installer" { "Installer".into() } else { ext },
                total,
                date,
                date_kind,
                detail: String::new(),
                category: c.category.into(),
                action: c.action.into(),
                tool: None,
                junk: vec![],
                path,
            }
        })
        .collect()
}

fn developer(conn: &Connection, home: &str) -> Vec<OldItem> {
    let mut out = Vec::new();
    let inv = crate::xcode::inventory(Path::new(home));
    for it in inv.items {
        let (label, date, date_kind) = match it.group.as_str() {
            "simulators" => ("Simulator", it.last_booted.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.timestamp()), "booted"),
            "support" => ("Device support", it.modified.map(|m| m as i64), "added"),
            "archives" => ("Xcode archive", it.modified.map(|m| m as i64), "created"),
            _ => continue,
        };
        let category = if it.blocked.is_some() { "protected" } else { "review" };
        out.push(OldItem {
            name: it.name,
            lane: "developer",
            label: label.into(),
            total: it.total,
            date,
            date_kind,
            detail: it.subtitle,
            category: category.into(),
            action: "quarantine".into(),
            tool: None,
            junk: vec![],
            path: it.path,
        });
    }
    // Language runtimes and toolchains: one entry per installed version.
    let q = "SELECT e.path, e.total FROM entries e LEFT JOIN entries p ON p.id = e.parent WHERE e.tag = 'unused-runtime' AND e.kind = 'directory' AND (p.tag IS NULL OR p.tag != e.tag)";
    if let Ok(mut st) = conn.prepare(q) {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))) {
            for (path, total) in rows.flatten() {
                let c = classify(&path, home);
                if c.tag != "unused-runtime" || !path.starts_with(home) {
                    continue;
                }
                let label = if path.contains("/.nvm/") || path.contains("/.fnm/") || path.contains("/.volta/") { "Node.js" } else if path.contains("/.rustup/") { "Rust toolchain" } else if path.contains("/.pyenv/") || path.contains("/uv/python/") { "Python" } else if path.contains("/uv/tools/") { "uv tool" } else if path.contains("/Android/") { "Android SDK" } else { "Runtime" };
                out.push(OldItem {
                    name: name_of(&path),
                    lane: "developer",
                    label: label.into(),
                    total,
                    date: mtime(Path::new(&path)),
                    date_kind: "installed",
                    detail: c.reason,
                    category: c.category.into(),
                    action: c.action.into(),
                    tool: c.tool,
                    junk: vec![],
                    path,
                });
            }
        }
    }
    out
}

fn backups(conn: &Connection, home: &str) -> Vec<OldItem> {
    let dir = format!("{}/Library/Application Support/MobileSync/Backup", home);
    let Ok(rd) = fs::read_dir(&dir) else { return vec![] };
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let p = e.path();
            let path = p.to_string_lossy().to_string();
            let info = plist::Value::from_file(p.join("Info.plist")).ok();
            let dict = info.as_ref().and_then(|v| v.as_dictionary());
            let device = dict.and_then(|d| d.get("Device Name")).and_then(|v| v.as_string()).unwrap_or("iPhone or iPad").to_string();
            let product = dict.and_then(|d| d.get("Product Name")).and_then(|v| v.as_string()).unwrap_or("").to_string();
            let date = dict
                .and_then(|d| d.get("Last Backup Date"))
                .and_then(|v| v.as_date())
                .map(|d| std::time::SystemTime::from(d))
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .or_else(|| mtime(&p.join("Status.plist")));
            let c = classify(&path, home);
            OldItem {
                name: device,
                lane: "backup",
                label: "Device backup".into(),
                total: db_total(conn, &path).unwrap_or_else(|| crate::xcode::allocated_size(&p).0),
                date,
                date_kind: "backed up",
                detail: product,
                category: c.category.into(),
                action: c.action.into(),
                tool: None,
                junk: vec![],
                path,
            }
        })
        .collect()
}

pub fn timeline(home: &str) -> Result<Vec<OldItem>, String> {
    let home = home.trim_end_matches('/');
    // Apps and Xcode measure the disk themselves; run them beside the database queries.
    let ((apps, developer_items), scanned) = rayon::join(
        || rayon::join(|| apps(home), || crate::open_db().map(|c| developer(&c, home)).unwrap_or_default()),
        || -> Result<Vec<OldItem>, String> {
            let conn = &crate::open_db().ok_or("No scan database loaded")?;
            let junk_items: Vec<(String, u64)> = junk(conn, home, 1 << 20)?.into_iter().filter(|j| j.action == "quarantine").map(|j| (j.path, j.total)).collect();
            let mut items = projects(conn, home, &junk_items);
            // A project already accounts for the big files inside it.
            let roots: HashSet<String> = items.iter().map(|i| i.path.clone()).collect();
            items.extend(files_and_downloads(conn, home).into_iter().filter(|i| !has_ancestor(&i.path, &roots)));
            items.extend(backups(conn, home));
            Ok(items)
        },
    );
    let mut items = apps;
    items.extend(scanned?);
    items.extend(developer_items);
    // Nothing is listed twice: a download folder already counts the files inside it.
    let mut seen = HashSet::new();
    let all: HashSet<String> = items.iter().filter(|i| i.lane != "app").map(|i| i.path.clone()).collect();
    items.retain(|i| seen.insert(i.path.clone()) && (i.lane == "app" || !has_ancestor(&i.path, &all)) && i.total > 0);
    items.sort_by(|a, b| b.total.cmp(&a.total));
    Ok(items)
}

