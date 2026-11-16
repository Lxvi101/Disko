//! A small reference walker that drives the matcher exactly the way the engine does (siblings
//! from the parent listing, no nesting below a match, files of at least 1 MiB). The engine has its
//! own walker; this one exists for the benchmark example and tests.

use crate::util::{du, Size, MB};
use crate::Clutter;
use disko_core::{Finding, Matched, WalkMatcher};
use rayon::prelude::*;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct WalkStats {
    pub dirs: AtomicU64,
    pub files: AtomicU64,
    pub bytes: AtomicU64,
}

/// Walk `root` (one volume, no symlinks) and send every finding the matcher produces to `sink`.
pub fn walk(clutter: &Clutter, root: &Path, sink: &(dyn Fn(Finding) + Sync)) -> WalkStats {
    let stats = WalkStats::default();
    if let Ok(m) = fs::symlink_metadata(root) {
        visit(clutter, root, m.dev(), sink, &stats);
    }
    stats
}

fn visit(c: &Clutter, dir: &Path, dev: u64, sink: &(dyn Fn(Finding) + Sync), st: &WalkStats) {
    st.dirs.fetch_add(1, Ordering::Relaxed);
    let Ok(rd) = fs::read_dir(dir) else { return };
    let entries: Vec<(String, std::path::PathBuf, fs::Metadata)> =
        rd.filter_map(Result::ok).filter_map(|e| Some((e.file_name().to_string_lossy().to_string(), e.path(), fs::symlink_metadata(e.path()).ok()?))).collect();
    let names: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
    for (name, path, m) in &entries {
        if m.is_file() {
            let bytes = m.blocks() * 512;
            st.files.fetch_add(1, Ordering::Relaxed);
            st.bytes.fetch_add(bytes, Ordering::Relaxed);
            if bytes >= MB {
                if let Some(tag) = c.on_file(path, name, bytes, m.mtime()) {
                    if let Some(f) = c.on_matched(Matched { tag, path: path.to_string_lossy().into(), bytes, files: 1, newest: Some(m.mtime()) }) {
                        sink(f);
                    }
                }
            }
        }
    }
    entries.par_iter().filter(|(_, _, m)| m.is_dir() && m.dev() == dev).for_each(|(name, path, _)| {
        if let Some(tag) = c.on_dir(path, name, &names) {
            let Size { bytes, files, newest } = du(path);
            st.bytes.fetch_add(bytes, Ordering::Relaxed);
            if let Some(f) = c.on_matched(Matched { tag, path: path.to_string_lossy().into(), bytes, files, newest }) {
                sink(f);
            }
        } else {
            visit(c, path, dev, sink, st);
        }
    });
}
