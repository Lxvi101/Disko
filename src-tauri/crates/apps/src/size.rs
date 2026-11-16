//! Parallel "du": allocated bytes and newest mtime of a path, cached for a few minutes so the
//! apps and sessions lists come back in seconds on repeat calls.
//!
//! Same rules as `disko_core::ops::allocated`: symlinks are never followed, files with several
//! hard links are counted once per measurement.

use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    /// Newest modification time below (and of) the path, unix seconds.
    pub newest: Option<i64>,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.bytes += other.bytes;
        self.newest = max_opt(self.newest, other.newest);
    }
}

pub fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

const TTL: Duration = Duration::from_secs(300);

fn cache() -> &'static Mutex<HashMap<PathBuf, (Instant, Usage)>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, (Instant, Usage)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Forget cached sizes (all, or at/under `path`). Call after removals.
pub fn invalidate(path: Option<&Path>) {
    let mut c = cache().lock().unwrap();
    match path {
        None => c.clear(),
        Some(p) => c.retain(|k, _| !(k.starts_with(p) || p.starts_with(k))),
    }
}

/// Cached measurement.
pub fn usage(path: &Path) -> Usage {
    if let Some((at, u)) = cache().lock().unwrap().get(path) {
        if at.elapsed() < TTL {
            return *u;
        }
    }
    let u = measure(path);
    cache().lock().unwrap().insert(path.to_path_buf(), (Instant::now(), u));
    u
}

pub fn bytes(path: &Path) -> u64 {
    usage(path).bytes
}

/// Uncached measurement, parallel across directories.
pub fn measure(path: &Path) -> Usage {
    let seen = Mutex::new(HashSet::new());
    walk(path, &seen)
}

fn mtime(m: &fs::Metadata) -> i64 {
    m.mtime()
}

fn walk(p: &Path, seen: &Mutex<HashSet<(u64, u64)>>) -> Usage {
    let Ok(m) = fs::symlink_metadata(p) else { return Usage::default() };
    if m.nlink() > 1 && !m.is_dir() && !seen.lock().unwrap().insert((m.dev(), m.ino())) {
        return Usage::default();
    }
    let mut u = Usage { bytes: m.blocks() * 512, newest: Some(mtime(&m)) };
    if m.is_dir() {
        let Ok(rd) = fs::read_dir(p) else { return u };
        let mut subdirs = Vec::new();
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                subdirs.push(e.path());
            } else if let Ok(em) = e.metadata() {
                // DirEntry::metadata doesn't follow symlinks.
                if em.nlink() > 1 && !seen.lock().unwrap().insert((em.dev(), em.ino())) {
                    continue;
                }
                u.bytes += em.blocks() * 512;
                u.newest = max_opt(u.newest, Some(mtime(&em)));
            }
        }
        let sub: Vec<Usage> = if subdirs.len() > 1 {
            subdirs.par_iter().map(|d| walk(d, seen)).collect()
        } else {
            subdirs.iter().map(|d| walk(d, seen)).collect()
        };
        for s in sub {
            u.add(s);
        }
    }
    u
}

/// Sum of several paths, measured in parallel (each cached on its own).
pub fn usage_all(paths: &[String]) -> Usage {
    let parts: Vec<Usage> = paths.par_iter().map(|p| usage(Path::new(p))).collect();
    let mut u = Usage::default();
    for p in parts {
        u.add(p);
    }
    u
}

/// Modification time of the path itself (not below), unix seconds.
pub fn own_mtime(path: &Path) -> Option<i64> {
    fs::symlink_metadata(path).ok().map(|m| m.mtime())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_core_allocated_and_counts_hardlinks_once() {
        let d = crate::util::test_dir("size");
        fs::create_dir_all(d.join("a/b/c")).unwrap();
        fs::write(d.join("a/b/c/f"), vec![1u8; 100_000]).unwrap();
        fs::write(d.join("a/g"), vec![1u8; 50_000]).unwrap();
        fs::hard_link(d.join("a/g"), d.join("a/b/h")).unwrap();
        std::os::unix::fs::symlink("/Applications", d.join("a/link")).unwrap();
        let u = measure(&d);
        assert_eq!(u.bytes, disko_core::ops::allocated(&d));
        assert!(u.bytes >= 150_000 && u.bytes < 300_000);
        assert!(u.newest.is_some());
        fs::remove_dir_all(&d).unwrap();
    }
}
