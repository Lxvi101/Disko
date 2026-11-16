//! Findings built from the scan database: disposable "junk" folders and duplicate files.

use crate::classify::classify;
use rayon::prelude::*;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::Hasher;
use std::io::{Read, Seek, SeekFrom};

/// Tags that describe generated or disposable data: dependencies, build output, caches, logs, installers.
const JUNK_TAGS: &[&str] = &[
    "generated-project-output", "generated-environment", "dependency-cache",
    "build-output", "build-cache", "project-cache", "xcode-cache",
    "app-cache", "browser-cache", "simulator-cache", "managed-cleanup", "model-cache",
    "diagnostic", "stale-log", "old-installer",
];

/// Home-relative folders whose tag spans many unrelated apps or projects; their children are listed instead.
const BROAD: &[&str] = &[
    "Library/Caches", ".cache", "Library/Logs", "Library/HTTPStorages",
    "Library/Developer/Xcode/DerivedData", "Library/Developer/CoreSimulator/Caches",
];

#[derive(Serialize, Clone, Debug)]
pub struct JunkItem {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub total: u64,
    pub mtime: Option<i64>,
    pub category: String,
    pub reason: String,
    pub tag: String,
    pub action: String,
    pub cost: String,
    pub tool: Option<String>,
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn secs(ns: Option<i64>) -> Option<i64> {
    ns.filter(|n| *n > 0).map(|n| n / 1_000_000_000)
}

/// True when any proper ancestor of `path` is in `set`.
pub(crate) fn has_ancestor(path: &str, set: &HashSet<String>) -> bool {
    let mut p = path;
    while let Some(i) = p.rfind('/') {
        p = &p[..i];
        if p.is_empty() {
            break;
        }
        if set.contains(p) {
            return true;
        }
    }
    false
}

/// The outermost folder (or file) of every junk tree, e.g. each `node_modules` but not the ones nested inside it.
pub fn junk(conn: &Connection, home: &str, min: u64) -> Result<Vec<JunkItem>, String> {
    let tags = JUNK_TAGS.iter().map(|t| format!("'{}'", t)).collect::<Vec<_>>().join(",");
    // An entry starts a junk tree when its parent carries a different tag.
    let sql = format!(
        "SELECT e.id, e.path, e.kind, e.total, e.mtime_ns FROM entries e LEFT JOIN entries p ON p.id = e.parent
         WHERE e.tag IN ({tags}) AND e.total >= ?1 AND e.error IS NULL AND (p.tag IS NULL OR p.tag != e.tag)
         ORDER BY e.total DESC LIMIT 20000"
    );
    let home = home.trim_end_matches('/');
    let broad: HashSet<String> = BROAD.iter().map(|b| format!("{}/{}", home, b)).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let roots: Vec<(i64, String, String, i64, Option<i64>)> = stmt
        .query_map([min as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<i64>>(3)?.unwrap_or(0), r.get(4)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let mut rows: Vec<(String, String, u64, Option<i64>)> = Vec::new();
    let mut children = conn
        .prepare("SELECT path, kind, total, mtime_ns FROM entries WHERE parent = ?1 AND total >= ?2 AND error IS NULL")
        .map_err(|e| e.to_string())?;
    for (id, path, kind, total, mtime) in roots {
        if broad.contains(&path) {
            let kids = children
                .query_map(rusqlite::params![id, min as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get::<_, Option<i64>>(3)?)))
                .map_err(|e| e.to_string())?;
            for (p, k, t, m) in kids.flatten() {
                rows.push((p, k, t.max(0) as u64, m));
            }
        } else {
            rows.push((path, kind, total.max(0) as u64, mtime));
        }
    }

    let chosen: HashSet<String> = rows.iter().map(|r| r.0.clone()).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, kind, total, mtime) in rows {
        if has_ancestor(&path, &chosen) || !seen.insert(path.clone()) {
            continue;
        }
        // Rules are applied live so a classifier update shows up without a rescan.
        let c = classify(&path, home);
        if !JUNK_TAGS.contains(&c.tag) || !(c.category == "rebuildable" || c.category == "review") {
            continue;
        }
        out.push(JunkItem {
            name: name_of(&path),
            path,
            kind,
            total,
            mtime: secs(mtime),
            category: c.category.to_string(),
            reason: c.reason,
            tag: c.tag.to_string(),
            action: c.action.to_string(),
            cost: c.cost.to_string(),
            tool: c.tool,
        });
    }
    out.sort_by(|a, b| b.total.cmp(&a.total));
    Ok(out)
}

#[derive(Serialize, Clone, Debug)]
pub struct DupFile {
    pub path: String,
    pub name: String,
    pub mtime: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DupGroup {
    /// Content size of one copy.
    pub size: u64,
    /// Space one copy takes on disk; removing all but one frees roughly this times (copies - 1).
    pub allocated: u64,
    pub files: Vec<DupFile>,
}

/// Only files people own and decide about; caches, repositories and app data are left alone.
const DUP_TAGS: &[&str] = &["user-files", "downloads", "old-installer", "user-data"];
/// Personal documents, media and downloads. Anything else is likely part of a project or an app.
const DUP_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "gif", "tif", "tiff", "webp", "raw", "dng", "cr2", "cr3", "nef", "arw", "psd", "ai", "sketch", "fig",
    "mp4", "mov", "m4v", "avi", "mkv", "webm", "mp3", "m4a", "wav", "aif", "aiff", "flac", "aac",
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "key", "pages", "numbers", "rtf", "txt", "csv", "epub",
    "zip", "rar", "7z", "tar", "gz", "tgz", "dmg", "pkg", "iso", "xip",
];
/// Folder names that hold generated or vendored copies, where duplicates are expected.
const DUP_SKIP_DIRS: &[&str] = &["build", "dist", "out", "vendor", "Pods", "SourcePackages", "DerivedData", "node_modules", "site-packages"];

/// A path whose folders are all ordinary user folders: no hidden folders, bundles, packages or build trees.
pub(crate) fn in_user_space(path: &str, home: &str) -> bool {
    let rel = path.strip_prefix(home).unwrap_or(path);
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let Some((_, dirs)) = parts.split_last() else { return false };
    // Hidden folders, bundles and packages belong to tools and apps.
    !dirs.iter().any(|seg| seg.starts_with('.') || DUP_SKIP_DIRS.contains(seg) || seg.rsplit_once('.').is_some_and(|(_, e)| !e.is_empty() && e.len() <= 12 && e.chars().all(|c| c.is_ascii_alphabetic())))
}

fn personal_file(path: &str, home: &str) -> bool {
    let ext = path.rsplit('/').next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    in_user_space(path, home) && DUP_EXT.contains(&ext.as_str())
}
const SAMPLE: u64 = 64 * 1024;

fn sample_hash(path: &str, size: u64) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut h = DefaultHasher::new();
    let mut buf = vec![0u8; SAMPLE as usize];
    let n = f.read(&mut buf).ok()?;
    h.write(&buf[..n]);
    if size > SAMPLE * 2 {
        f.seek(SeekFrom::Start(size / 2)).ok()?;
        let n = f.read(&mut buf).ok()?;
        h.write(&buf[..n]);
        f.seek(SeekFrom::Start(size - SAMPLE)).ok()?;
        let n = f.read(&mut buf).ok()?;
        h.write(&buf[..n]);
    }
    Some(h.finish())
}

fn full_hash(path: &str) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut h = DefaultHasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return Some(h.finish());
        }
        h.write(&buf[..n]);
    }
}

/// Byte-for-byte comparison, so a hash collision can never be reported as a duplicate.
fn same_bytes(a: &str, b: &str) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (File::open(a), File::open(b)) else { return false };
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let Ok(na) = fa.read(&mut ba) else { return false };
        if na == 0 {
            return fb.read(&mut bb[..1]).map(|n| n == 0).unwrap_or(false);
        }
        if fb.read_exact(&mut bb[..na]).is_err() || ba[..na] != bb[..na] {
            return false;
        }
    }
}

type Cand = (String, u64, u64, Option<i64>);

/// Split each group by `key`, keeping only sub-groups that still hold two or more files.
fn refine(groups: Vec<Vec<Cand>>, key: impl Fn(&Cand) -> Option<u64> + Sync) -> Vec<Vec<Cand>> {
    groups
        .into_par_iter()
        .flat_map_iter(|g| {
            let keyed: Vec<(Option<u64>, Cand)> = g.into_par_iter().map(|c| (key(&c), c)).collect();
            let mut by: HashMap<u64, Vec<Cand>> = HashMap::new();
            for (k, c) in keyed {
                if let Some(k) = k {
                    by.entry(k).or_default().push(c);
                }
            }
            by.into_values().filter(|v| v.len() > 1).collect::<Vec<_>>()
        })
        .collect()
}

pub fn duplicates(conn: &Connection, home: &str, min: u64) -> Result<Vec<DupGroup>, String> {
    let home = home.trim_end_matches('/');
    let mut stmt = conn
        .prepare("SELECT path, logical, allocated, device, inode, mtime_ns FROM entries WHERE kind = 'file' AND logical >= ?1 AND error IS NULL")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([min as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as u64,
                r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
                r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    // Same size is the cheap first filter; hard links (same inode) are one file, not two.
    let mut by_size: HashMap<u64, Vec<Cand>> = HashMap::new();
    let mut inodes: HashSet<(i64, i64)> = HashSet::new();
    for (path, logical, allocated, dev, ino, mtime) in rows.flatten() {
        if ino != 0 && !inodes.insert((dev, ino)) {
            continue;
        }
        if !personal_file(&path, home) {
            continue;
        }
        let c = classify(&path, home);
        if !DUP_TAGS.contains(&c.tag) || c.action != "quarantine" {
            continue;
        }
        by_size.entry(logical).or_default().push((path, logical, allocated, secs(mtime)));
    }
    let mut groups: Vec<Vec<Cand>> = by_size.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g[0].1));
    groups.truncate(5000);

    let groups = refine(groups, |c| sample_hash(&c.0, c.1));
    let groups = refine(groups, |c| full_hash(&c.0));

    let mut out: Vec<DupGroup> = groups
        .into_par_iter()
        .filter_map(|mut g| {
            let first = g[0].0.clone();
            g.retain(|c| c.0 == first || same_bytes(&first, &c.0));
            if g.len() < 2 {
                return None;
            }
            g.sort_by_key(|c| c.3.unwrap_or(i64::MAX));
            Some(DupGroup {
                size: g[0].1,
                allocated: g.iter().map(|c| c.2).max().unwrap_or(g[0].1),
                files: g.into_iter().map(|(path, _, _, mtime)| DupFile { name: name_of(&path), path, mtime }).collect(),
            })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.allocated * (g.files.len() as u64 - 1)));
    out.truncate(500);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestors_are_found() {
        let set: HashSet<String> = ["/u/p/node_modules".to_string()].into_iter().collect();
        assert!(has_ancestor("/u/p/node_modules/x/node_modules", &set));
        assert!(!has_ancestor("/u/p/node_modules", &set));
        assert!(!has_ancestor("/u/p/node_modules2/x", &set));
    }

    #[test]
    fn personal_files_only() {
        let h = "/Users/a";
        assert!(personal_file("/Users/a/Downloads/Setup.dmg", h));
        assert!(personal_file("/Users/a/Pictures/trip/IMG_1.HEIC", h));
        assert!(!personal_file("/Users/a/Code/x/build/icon.png", h));
        assert!(!personal_file("/Users/a/Code/x/.git/objects/pack/p.pack", h));
        assert!(!personal_file("/Users/a/Tools/Foo.app/Contents/icon.png", h));
        assert!(!personal_file("/Users/a/Downloads/notes.bin", h));
    }

    #[test]
    fn byte_comparison() {
        let dir = std::env::temp_dir().join(format!("disko-dup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        std::fs::write(&a, vec![7u8; 300_000]).unwrap();
        std::fs::write(&b, vec![7u8; 300_000]).unwrap();
        let mut other = vec![7u8; 300_000];
        other[150_001] = 8;
        std::fs::write(&c, other).unwrap();
        let s = |p: &std::path::PathBuf| p.to_string_lossy().to_string();
        assert!(same_bytes(&s(&a), &s(&b)));
        assert!(!same_bytes(&s(&a), &s(&c)));
        assert_eq!(full_hash(&s(&a)), full_hash(&s(&b)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
