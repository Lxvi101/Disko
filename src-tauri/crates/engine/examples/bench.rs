//! Real-disk benchmark: the new engine vs 0.6's scanner traversal.
//!
//! ```text
//! flock /tmp/fd-build.lock cargo run --release -p disko-engine -j 4 --example bench -- new [root]
//! flock /tmp/fd-build.lock cargo run --release -p disko-engine -j 4 --example bench -- legacy [root]
//! ```
//!
//! "legacy" is 0.6's `scanner.rs` walk + collector (thorough mode: one entry per file) using its
//! own `bulkdir.rs`, *without* the SQLite write that followed it in 0.6 — so it is a lower bound
//! for 0.6's scan time. Each mode reports wall time, files/s and peak RSS.

#[path = "../../../../legacy/rust/bulkdir.rs"]
#[allow(dead_code)]
mod bulkdir;

use disko_core::{Category, WalkMatcher};
use disko_engine::{Engine, EngineConfig, EngineEvent};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn peak_rss() -> u64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    u.ru_maxrss as u64 // bytes on macOS
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

/// A matcher shaped like the clutter crate's: a few name rules.
struct Bench;
impl WalkMatcher for Bench {
    fn on_dir(&self, _: &Path, name: &str, siblings: &[&str]) -> Option<u32> {
        match name {
            "node_modules" => Some(1),
            "DerivedData" | ".next" | ".gradle" => Some(2),
            "target" if siblings.contains(&"Cargo.toml") => Some(2),
            "__pycache__" | ".venv" => Some(3),
            _ => None,
        }
    }
    fn on_file(&self, _: &Path, name: &str, _: u64, _: i64) -> Option<u32> {
        (name.ends_with(".dmg") || name.ends_with(".pkg")).then_some(4)
    }
    fn category(&self, tag: u32) -> Category {
        match tag {
            1 | 2 | 3 => Category::Build,
            _ => Category::Installers,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("new");
    let root = PathBuf::from(args.get(2).map(|s| s.as_str()).unwrap_or("/"));
    let fda = disko_engine::has_full_disk_access();
    println!("root {}  full disk access: {}", root.display(), fda);
    match mode {
        "legacy" => legacy(&root, fda),
        _ => new(&root),
    }
}

fn new(root: &Path) {
    let dir = std::env::temp_dir().join("disko-bench");
    let _ = std::fs::create_dir_all(&dir);
    let snap = dir.join("snapshot.bin");
    let _ = std::fs::remove_file(&snap);
    let mut cfg = EngineConfig { root: root.to_path_buf(), ..Default::default() };
    if let Some(n) = std::env::var("DISKO_THREADS").ok().and_then(|v| v.parse().ok()) {
        cfg.threads = n;
    }
    println!("threads {}", cfg.threads);
    let engine = Engine::with_config(snap.clone(), cfg.clone());
    let (progress, trees, matched) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let first_tree_ms = AtomicU64::new(0);
    let start = Instant::now();
    let sink = |e: EngineEvent| match e {
        EngineEvent::Progress(_) => {
            progress.fetch_add(1, Ordering::Relaxed);
        }
        EngineEvent::Tree(_) => {
            if trees.fetch_add(1, Ordering::Relaxed) == 0 {
                first_tree_ms.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
            }
        }
        EngineEvent::Matched(_) => {
            matched.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    };
    let st = engine.scan(Arc::new(Bench), Arc::new(AtomicBool::new(false)), &sink).unwrap();
    let wall = start.elapsed();
    let secs = st.elapsed_ms as f64 / 1000.0;
    println!("new: scan {:.2}s (incl. snapshot save: {:.2}s wall)", secs, wall.as_secs_f64());
    println!("  files {}  dirs {}  bytes {}  files/s {:.0}  entries/s {:.0}", st.files, st.dirs, mb(st.bytes), st.files as f64 / secs, (st.files + st.dirs) as f64 / secs);
    println!("  listing {} ms total, tree-lock wait {} ms total", st.list_ms, st.lock_wait_ms);
    println!("  nodes {}  unreadable {}  tree heap {}  snapshot {}  peak RSS {}", st.nodes, st.unreadable, mb(st.tree_bytes), mb(st.snapshot_bytes), mb(peak_rss()));
    println!("  events: progress {}  tree {} (first at {} ms)  matched {}", progress.load(Ordering::Relaxed), trees.load(Ordering::Relaxed), first_tree_ms.load(Ordering::Relaxed), matched.load(Ordering::Relaxed));

    let t = Instant::now();
    let v = engine.volume();
    println!("volume ({} ms): total {} used {} free {} purgeable {} scanned {}", t.elapsed().as_millis(), mb(v.total), mb(v.used), mb(v.free), mb(v.purgeable), mb(v.scanned));
    for h in &v.hidden {
        println!("  hidden {:<10} {:>12}  {} — {}", h.kind, mb(h.bytes), h.label, h.why);
    }
    let t = Instant::now();
    let top = engine.tree(None, 3).unwrap();
    println!("tree(/, 3) in {} ms: {} children", t.elapsed().as_millis(), top.children.as_ref().map(|c| c.len()).unwrap_or(0));
    for c in top.children.unwrap_or_default().iter().take(8) {
        println!("  {:>12}  {}", mb(c.bytes), c.path);
    }
    let t = Instant::now();
    let s = engine.search("node_modules", 20);
    println!("search(node_modules) {} hits in {} ms", s.len(), t.elapsed().as_millis());
    let t = Instant::now();
    let l = engine.largest_files(None, 20);
    println!("largest_files {} in {} ms; biggest {:?}", l.len(), t.elapsed().as_millis(), l.first().map(|n| (&n.path, mb(n.bytes))));
    drop(engine);

    let rss_before = peak_rss();
    let t = Instant::now();
    let e2 = Engine::with_config(snap.clone(), cfg);
    let age = e2.load_snapshot();
    println!("load_snapshot: {} ms (age {:?}), needs_scan {}, root bytes {}, peak RSS {}", t.elapsed().as_millis(), age, e2.needs_scan(), mb(e2.size_of("/").map(|n| n.bytes).unwrap_or(0)), mb(rss_before.max(peak_rss())));
    let t = Instant::now();
    let path = e2.largest_files(None, 1).first().map(|n| n.path.clone());
    if let Some(p) = path {
        // Simulated removal (tree only; nothing on disk is touched — the path still exists, so
        // removed() re-lists it instead; use a fake child path to exercise the O(depth) path).
        let fake = format!("{}/does-not-exist-{}", Path::new(&p).parent().unwrap().display(), std::process::id());
        let changed = e2.removed(&[fake]);
        println!("removed(fake) {:?} in {} µs", changed, t.elapsed().as_micros());
    }
}

// ---------------------------------------------------------------- legacy (0.6) traversal

#[allow(dead_code)]
struct RawChild {
    name: String,
    token: u64,
    kind: u8,
    dev: u64,
    ino: u64,
    mode: u32,
    size: u64,
    mtime_ns: i64,
    ctime_ns: i64,
    allocated: u64,
    nlink: u64,
    error: Option<String>,
}

#[allow(dead_code)]
struct Entry {
    name: Box<str>,
    parent: u32,
    kind: u8,
    dev: u64,
    ino: u64,
    mode: u32,
    size: u64,
    mtime_ns: i64,
    ctime_ns: i64,
    allocated: u64,
    logical: u64,
    total: u64,
    error: Option<Box<str>>,
    small_count: u32,
    top: u32,
    sub: u32,
    junk: u8,
}

const TCC_GATED: [&str; 26] = [
    "Library/Application Support/AddressBook", "Library/Application Support/CallHistoryDB", "Library/Application Support/CallHistoryTransactions", "Library/Application Support/com.apple.TCC",
    "Library/Application Support/com.apple.sharedfilelist", "Library/Application Support/Knowledge", "Library/Accounts", "Library/Assistant", "Library/Autosave Information", "Library/Biome",
    "Library/Calendars", "Library/Cookies", "Library/CoreFollowUp", "Library/Daemon Containers", "Library/DuetExpertCenter", "Library/HomeKit", "Library/IdentityServices", "Library/Mail",
    "Library/Messages", "Library/Metadata/CoreSpotlight", "Library/PersonalizationPortrait", "Library/Reminders", "Library/Safari", "Library/Sharing", "Library/Suggestions", "Library/Trial",
];

fn is_tcc_gated(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(".photoslibrary") || TCC_GATED.iter().any(|g| s.ends_with(&format!("/{}", g)))
}

struct Walker {
    skip_tcc: bool,
    tx: std::sync::mpsc::Sender<(u64, PathBuf, Vec<RawChild>, Option<String>)>,
    next_token: AtomicU64,
    devices: Vec<u64>,
    excludes: Vec<PathBuf>,
}

impl Walker {
    fn walk_dir<'s>(&'s self, scope: &rayon::Scope<'s>, dir: PathBuf, token: u64) {
        let listed = bulkdir::list(&dir).map(|list| {
            list.into_iter()
                .map(|e| RawChild { name: e.name, token: 0, kind: e.kind, dev: e.dev, ino: e.ino, mode: e.mode, size: e.size, mtime_ns: e.mtime_ns, ctime_ns: e.ctime_ns, allocated: e.allocated, nlink: e.nlink, error: e.error })
                .collect::<Vec<_>>()
        });
        let mut children = match listed {
            Ok(c) => c,
            Err(e) => {
                let _ = self.tx.send((token, dir, Vec::new(), Some(e.to_string())));
                return;
            }
        };
        let excludes: Vec<&PathBuf> = self.excludes.iter().filter(|x| x.starts_with(&dir) && x.as_path() != dir).collect();
        let all_excluded = self.excludes.iter().any(|x| dir.starts_with(x));
        let cloud_parent = dir.file_name().map(|n| n == "Library").unwrap_or(false);
        let mut subdirs = Vec::new();
        for c in children.iter_mut() {
            let excluded = all_excluded || (cloud_parent && c.kind == 0 && (c.name == "Mobile Documents" || c.name == "CloudStorage")) || (!excludes.is_empty() && excludes.iter().any(|x| x.as_path() == dir.join(&c.name)));
            if c.kind == 0 {
                let path = dir.join(&c.name);
                if excluded {
                    c.error = Some("Excluded from scan.".into());
                } else if self.skip_tcc && is_tcc_gated(&path) {
                    c.error = Some("Needs Full Disk Access.".into());
                } else if !self.devices.contains(&c.dev) {
                    c.error = Some("Different filesystem; not traversed.".into());
                } else if c.error.is_none() {
                    c.token = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
                    subdirs.push((path, c.token));
                }
            } else if excluded {
                c.error = Some("Excluded from scan.".into());
                c.allocated = 0;
            }
        }
        let _ = self.tx.send((token, dir, children, None));
        for (p, t) in subdirs {
            scope.spawn(move |s| self.walk_dir(s, p, t));
        }
    }
}

fn legacy(root: &Path, fda: bool) {
    use std::collections::{HashMap, HashSet};
    use std::os::unix::fs::MetadataExt;
    let start = Instant::now();
    let root_meta = std::fs::symlink_metadata(root).unwrap();
    let mut devices = vec![root_meta.dev() as u64];
    let whole = root == Path::new("/");
    if whole {
        for extra in ["/System/Volumes/Data", "/System/Volumes/VM"] {
            if let Ok(m) = std::fs::symlink_metadata(extra) {
                devices.push(m.dev() as u64);
            }
        }
    }
    let home = disko_core::paths::home();
    let mut excludes: Vec<PathBuf> = vec![home.join("Library/Mobile Documents"), home.join("Library/CloudStorage"), "/Volumes".into(), "/System/Volumes/Preboot".into(), "/System/Volumes/Update".into(), "/System/Volumes/Hardware".into(), "/System/Volumes/xarts".into(), "/System/Volumes/iSCPreboot".into(), "/dev".into()];
    if whole {
        if let Ok(list) = std::fs::read_to_string("/usr/share/firmlinks") {
            for line in list.lines() {
                if let Some((_, target)) = line.split_once('\t') {
                    excludes.push(PathBuf::from("/System/Volumes/Data").join(target.trim()));
                }
            }
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let walker = Walker { skip_tcc: !fda, tx, next_token: AtomicU64::new(1), devices, excludes };
    let root2 = root.to_path_buf();
    let traversal = std::thread::spawn(move || {
        let walker = walker;
        rayon::scope(|s| walker.walk_dir(s, root2, 0));
        drop(walker);
    });
    let mut entries: Vec<Entry> = vec![Entry { name: root.to_string_lossy().into(), parent: u32::MAX, kind: 0, dev: 0, ino: 0, mode: 0, size: 0, mtime_ns: 0, ctime_ns: 0, allocated: 0, logical: 0, total: 0, error: None, small_count: 0, top: u32::MAX, sub: u32::MAX, junk: 0 }];
    let mut token_index: HashMap<u64, u32> = HashMap::new();
    token_index.insert(0, 0);
    let mut inodes: HashSet<(u64, u64)> = HashSet::new();
    let (mut seen, mut bytes, mut dirs) = (1u64, 0u64, 0u64);
    for (parent_token, _dir, children, dir_error) in rx.iter() {
        dirs += 1;
        let Some(&pidx) = token_index.get(&parent_token) else { continue };
        if let Some(e) = dir_error {
            entries[pidx as usize].error = Some(e.into());
        }
        for c in children {
            seen += 1;
            let mut allocated = if c.error.is_some() { 0 } else { c.allocated };
            if c.kind == 1 && c.nlink > 1 && allocated > 0 && !inodes.insert((c.dev, c.ino)) {
                allocated = 0;
            }
            bytes += allocated;
            let idx = entries.len() as u32;
            if c.token != 0 {
                token_index.insert(c.token, idx);
            }
            entries.push(Entry { name: c.name.into(), parent: pidx, kind: c.kind, dev: c.dev, ino: c.ino, mode: c.mode, size: c.size, mtime_ns: c.mtime_ns, ctime_ns: c.ctime_ns, allocated, logical: 0, total: allocated, error: c.error.map(|e| e.into()), small_count: 0, top: 0, sub: 0, junk: 0 });
        }
    }
    let _ = traversal.join();
    for i in (1..entries.len()).rev() {
        let (t, p) = (entries[i].total, entries[i].parent as usize);
        entries[p].total += t;
    }
    let secs = start.elapsed().as_secs_f64();
    println!("legacy (walk + collect, no SQLite write): {:.2}s", secs);
    println!("  entries {}  dirs listed {}  bytes {}  entries/s {:.0}  rows {}  peak RSS {}", seen, dirs, mb(bytes), seen as f64 / secs, entries.len(), mb(peak_rss()));
    println!("  root total {}", mb(entries[0].total));
}
