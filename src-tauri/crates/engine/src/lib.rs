//! Disko's one and only scan: the whole boot disk, walked once, kept in memory as a compact tree,
//! saved as one small snapshot, and kept current with FSEvents.
//!
//! # API
//!
//! ```ignore
//! let engine = Engine::new(disko_core::paths::cache_dir().join("snapshot.bin"));
//! let age = engine.load_snapshot();            // instant map at launch (None = no usable snapshot)
//! let watch = engine.watch(matcher.clone(), sink.clone()); // replays changes since the snapshot
//! if engine.needs_scan() {
//!     engine.scan(matcher, cancel, &*sink)?;   // blocking; streams EngineEvents; saves the snapshot
//! }
//! engine.tree(None, 3);                        // views for the UI / MCP
//! engine.removed(&paths);                      // after Disko deleted/trashed something: instant
//! ```
//!
//! | Call | What |
//! |---|---|
//! | [`Engine::new`] / [`Engine::with_config`] | Engine for "/" (or a configured root). Cheap. |
//! | [`Engine::load_snapshot`] | Load the snapshot file; returns its age. |
//! | [`Engine::needs_scan`] | True without a tree, or when the snapshot can't be brought up to date from FSEvents history. |
//! | [`Engine::scan`] | The one whole-disk scan. Blocking; streams `Progress` (~4/s), `Tree` (~1/s, only when no earlier tree is shown), `Matched`, `Volume`; ends with `Tree` + `Changed([root])`; saves the snapshot. |
//! | [`Engine::tree`] | Node with `depth` levels, largest first, small children folded into one `kind: "other"` child (named "N small items", path = the folder's). |
//! | [`Engine::list`] | One folder's children, nothing folded except its own small files. |
//! | [`Engine::search`] | Case-insensitive name search over folders and kept files, largest first. |
//! | [`Engine::largest_files`] | Biggest kept files (≥ `min_file`), optionally below a folder. |
//! | [`Engine::size_of`] | One node without children. |
//! | [`Engine::matched_at`] | Current totals of the matched folders at/above the given paths. |
//! | [`Engine::matched_under`] | Every matched folder/file at/below a path (None = all): rebuild findings after `load_snapshot`. |
//! | [`Engine::removed`] | Drop paths after a removal: O(depth) per path, returns changed folders. |
//! | [`Engine::refresh`] | Re-list folders (new subfolders are walked). Returns changed folders. |
//! | [`Engine::watch`] | FSEvents → `refresh` → `Changed` (+ `Matched`/`Gone`/`Volume`, or `NeedsScan`). |
//! | [`Engine::save_snapshot`] | Write the snapshot (atomically). Call on quit and after big changes. |
//! | [`Engine::volume`] | Total/used/free/purgeable, scanned bytes and the hidden-space breakdown. |
//!
//! "Changed" paths are the deepest folders whose totals changed; their ancestors changed too.
//!
//! # What the scan covers
//!
//! "/" with the APFS data volume through its firmlinks (twins under `/System/Volumes/Data` are
//! skipped, so nothing counts twice). Other volumes (Preboot, Recovery, VM, Update, /Volumes …),
//! iCloud Drive (`Library/Mobile Documents`) and File Provider folders (`Library/CloudStorage`)
//! are not walked; `volume()` accounts for them. Without Full Disk Access, privacy-gated folders
//! (Mail, Safari, Messages …, and other apps' containers, which would raise the "data from other
//! apps" prompt) are not listed and show "Needs Full Disk Access". ~/Desktop, ~/Documents and
//! ~/Downloads are walked (`user_folders`; macOS asks once per folder).
//! Sizes are allocated bytes. Hard links count once (in the folder that saw them first); APFS
//! clone families (`cp -c`, Finder duplicates, pnpm/Xcode stores) count their shared data once,
//! the other (pure) clones count 0 — so the map adds up to what the disk uses.
//!
//! # Storage
//!
//! Every folder is kept; files of at least `min_file` (1 MiB) are kept individually; smaller files
//! are folded into their folder (`own` count, bytes, newest mtime). Struct-of-arrays with a name
//! arena: about 55 bytes + name per node.
//!
//! # Live updates and relaunch
//!
//! The snapshot stores the FSEvents event id taken right before the walk (advanced as changes are
//! applied) and the event history's UUID for the device. On launch, `watch()` starts the stream
//! *at that id*, so the system replays every folder that changed while Disko wasn't running and
//! only those are re-listed. If the history can't be used (UUID changed, events dropped,
//! "must scan subdirs" for the root, snapshot older than `max_snapshot_age`) the engine reports it
//! through `needs_scan()` / `EngineEvent::NeedsScan` and the app runs the one full scan.

mod bulk;
mod ffi;
mod snapshot;
mod tree;
mod volume;
mod walk;
mod watch;

#[cfg(test)]
mod testutil;

use disko_core::{Category, Matched, ScanProgress, TreeNode, VolumeInfo, WalkMatcher};
use std::collections::{BTreeSet, BinaryHeap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tree::*;
use walk::{Job, Scope, Stats, Walk};

pub use walk::has_full_disk_access;
pub use watch::{debug_current_event_id, debug_raw_events, WatchHandle};

/// What the engine tells the app while scanning and watching.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    Progress(ScanProgress),
    /// Root, depth 3, partial totals while walking; once more at the end.
    Tree(TreeNode),
    /// A matched folder's subtree is complete (or a matched file was seen / changed).
    Matched(Matched),
    /// Deepest folders whose totals changed (removal, refresh, FSEvents).
    Changed(Vec<String>),
    /// Matched folders/files that no longer exist (deleted outside Disko).
    Gone(Vec<String>),
    Volume(VolumeInfo),
    /// The tree can't be kept current from FSEvents (events dropped, history reset): run `scan`.
    NeedsScan(String),
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// What to scan. "/" = the whole boot disk.
    pub root: PathBuf,
    /// Files at least this big are kept as their own nodes.
    pub min_file: u64,
    /// Walker threads (default: one per CPU, at least 4). The walk is bound by the latency of
    /// `getattrlistbulk`, not by CPU: on an 8-core M2 eight walkers keep ~3.6 cores busy on
    /// average, so more than two cores stay free while the scan finishes as fast as possible.
    pub threads: usize,
    /// Without Full Disk Access, still walk ~/Desktop, ~/Documents and ~/Downloads (macOS asks
    /// once per folder and remembers). Default true: most projects live there.
    pub user_folders: bool,
    /// Without Full Disk Access, also list other apps' containers (~/Library/Containers, Group
    /// Containers), which raises macOS's "data from other apps" prompt. Default false.
    pub allow_prompts: bool,
    /// Extra absolute paths not to walk.
    pub excludes: Vec<PathBuf>,
    /// Older snapshots are shown but `needs_scan()` asks for a fresh scan.
    pub max_snapshot_age: Duration,
    /// Override the Full Disk Access probe (tests).
    pub assume_fda: Option<bool>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        EngineConfig {
            root: PathBuf::from("/"),
            min_file: 1 << 20,
            threads: cpus.max(4),
            user_folders: true,
            allow_prompts: false,
            excludes: Vec::new(),
            max_snapshot_age: Duration::from_secs(7 * 24 * 3600),
            assume_fda: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Nodes kept in the tree (folders + big files).
    pub nodes: u64,
    pub unreadable: u64,
    pub elapsed_ms: u64,
    /// Approximate heap held by the tree.
    pub tree_bytes: u64,
    pub snapshot_bytes: u64,
    /// Diagnostics: summed time walkers spent listing folders / waiting for the tree lock.
    pub list_ms: u64,
    pub lock_wait_ms: u64,
}

type Queue = Option<(Vec<(String, bool)>, u64)>;

pub struct Engine {
    cfg: EngineConfig,
    snapshot_path: PathBuf,
    pool: rayon::ThreadPool,
    scope: RwLock<Arc<Scope>>,
    current: RwLock<Tree>,
    building: RwLock<Option<Arc<RwLock<Tree>>>>,
    scan_lock: Mutex<()>,
    refresh_lock: Mutex<()>,
    /// Some while scanning: refreshes to apply to the new tree once it's swapped in.
    queued: Mutex<Queue>,
    matcher: Mutex<Option<Arc<dyn WalkMatcher>>>,
    resumable: AtomicBool,
    vcache: Mutex<volume::Cache>,
}

struct NoMatch;
impl WalkMatcher for NoMatch {
    fn on_dir(&self, _: &Path, _: &str, _: &[&str]) -> Option<u32> {
        None
    }
    fn on_file(&self, _: &Path, _: &str, _: u64, _: i64) -> Option<u32> {
        None
    }
    fn category(&self, _: u32) -> Category {
        Category::Cache
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Engine {
    /// Engine for the whole boot disk, keeping its snapshot at `snapshot_path`.
    pub fn new(snapshot_path: PathBuf) -> Arc<Engine> {
        Self::with_config(snapshot_path, EngineConfig::default())
    }

    pub fn with_config(snapshot_path: PathBuf, cfg: EngineConfig) -> Arc<Engine> {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(cfg.threads.max(1)).thread_name(|i| format!("disko-walk-{}", i)).build().expect("walker threads");
        let fda = cfg.assume_fda.unwrap_or_else(has_full_disk_access);
        let scope = Scope::new(&cfg, fda, &snapshot_path);
        let root = scope.root_str.clone();
        Arc::new(Engine {
            snapshot_path,
            pool,
            scope: RwLock::new(Arc::new(scope)),
            current: RwLock::new(Tree::new(&root, cfg.min_file)),
            building: RwLock::new(None),
            scan_lock: Mutex::new(()),
            refresh_lock: Mutex::new(()),
            queued: Mutex::new(None),
            matcher: Mutex::new(None),
            resumable: AtomicBool::new(false),
            vcache: Mutex::new(volume::Cache::default()),
            cfg,
        })
    }

    fn scope(&self) -> Arc<Scope> {
        self.scope.read().unwrap().clone()
    }

    pub(crate) fn watch_root(&self) -> String {
        self.scope().watch_root()
    }

    /// Whether Disko currently has Full Disk Access (best-effort probe).
    pub fn full_disk_access(&self) -> bool {
        self.cfg.assume_fda.unwrap_or_else(has_full_disk_access)
    }

    /// Device whose FSEvents history covers the tree (the data volume for "/").
    fn history_device(&self, scope: &Scope) -> u64 {
        let p = if scope.root_str == "/" { "/System/Volumes/Data" } else { scope.root_str.as_str() };
        std::fs::symlink_metadata(p).map(|m| m.dev() as u64).unwrap_or(0)
    }

    /// Run `f` on the tree queries should see: the current one, or the one being built when
    /// there is nothing else yet.
    fn with_view<R>(&self, f: impl FnOnce(&Tree) -> R) -> Option<R> {
        {
            let cur = self.current.read().unwrap();
            if !cur.is_empty() {
                return Some(f(&cur));
            }
        }
        let b = self.building.read().unwrap().clone()?;
        let t = b.read().unwrap();
        Some(f(&t))
    }

    // ---------------------------------------------------------------- snapshot

    /// Load the snapshot (if any and if no tree is loaded yet). Returns its age.
    pub fn load_snapshot(&self) -> Option<Duration> {
        let t = snapshot::load(&self.snapshot_path).ok()?;
        let scope = self.scope();
        if t.root != scope.root_str || t.min_file != self.cfg.min_file {
            return None;
        }
        let age = Duration::from_secs(now_secs().saturating_sub(t.created));
        let uuid = ffi::device_uuid(self.history_device(&scope));
        let resumable = t.complete && t.event_id > 0 && !t.uuid.is_empty() && t.uuid == uuid && age < self.cfg.max_snapshot_age;
        let mut cur = self.current.write().unwrap();
        if !cur.is_empty() {
            return None;
        }
        *cur = t;
        self.resumable.store(resumable, Ordering::Relaxed);
        Some(age)
    }

    /// True when there's no tree, or the loaded one can't be caught up from FSEvents history.
    pub fn needs_scan(&self) -> bool {
        self.current.read().unwrap().is_empty() || !self.resumable.load(Ordering::Relaxed)
    }

    /// Write the snapshot atomically. Returns the file size (0 when there's nothing to save).
    pub fn save_snapshot(&self) -> std::io::Result<u64> {
        let t = self.current.read().unwrap();
        if t.is_empty() || !t.complete {
            return Ok(0);
        }
        snapshot::save(&t, &self.snapshot_path)
    }

    // ---------------------------------------------------------------- scan

    /// The one scan. Blocks until the walk is done (or `cancel` is set) and streams events to
    /// `sink` from walker threads. A previous tree stays queryable until the new one is complete.
    pub fn scan(&self, matcher: Arc<dyn WalkMatcher>, cancel: Arc<AtomicBool>, sink: &(dyn Fn(EngineEvent) + Sync)) -> Result<ScanStats, String> {
        let _guard = self.scan_lock.try_lock().map_err(|_| "A scan is already running".to_string())?;
        *self.matcher.lock().unwrap() = Some(matcher.clone());
        let start = Instant::now();
        let fda = self.full_disk_access();
        let scope = Arc::new(Scope::new(&self.cfg, fda, &self.snapshot_path));
        *self.scope.write().unwrap() = scope.clone();

        let event_id = ffi::current_event_id();
        let uuid = ffi::device_uuid(self.history_device(&scope));
        let (had_tree, prev_nodes) = {
            let c = self.current.read().unwrap();
            (!c.is_empty(), c.live())
        };
        let used = volume::statfs(&scope.root_str).map(|(t, f, _)| t.saturating_sub(f)).unwrap_or(0).max(1);
        let mut t = Tree::new(&scope.root_str, self.cfg.min_file);
        t.reserve(prev_nodes + prev_nodes / 8);
        t.set_pending(0, 1);
        t.event_id = event_id;
        t.uuid = uuid;
        t.created = now_secs();
        let b = Arc::new(RwLock::new(t));
        *self.building.write().unwrap() = Some(b.clone());
        *self.queued.lock().unwrap() = Some((Vec::new(), 0));

        let stats = Stats::default();
        let done = AtomicBool::new(false);
        let progress = |phase: &str, current: String| ScanProgress {
            phase: phase.into(),
            files: stats.files.load(Ordering::Relaxed),
            dirs: stats.dirs.load(Ordering::Relaxed),
            bytes: stats.bytes.load(Ordering::Relaxed),
            elapsed_ms: start.elapsed().as_millis() as u64,
            current,
            fraction: if phase == "done" { 1.0 } else { (stats.bytes.load(Ordering::Relaxed) as f64 / used as f64).min(0.99) },
        };
        std::thread::scope(|ts| {
            ts.spawn(|| {
                sink(EngineEvent::Volume(self.volume()));
                let mut tick = 0u32;
                while !done.load(Ordering::Relaxed) {
                    for _ in 0..5 {
                        std::thread::sleep(Duration::from_millis(50));
                        if done.load(Ordering::Relaxed) {
                            return;
                        }
                    }
                    tick += 1;
                    let (cur, view) = {
                        let t = b.read().unwrap();
                        let last = stats.last_dir.load(Ordering::Relaxed);
                        let cur = if (last as usize) < t.len() { t.path(last) } else { String::new() };
                        let view = (!had_tree && tick % 4 == 0).then(|| t.view(0, 3));
                        (cur, view)
                    };
                    sink(EngineEvent::Progress(progress("walk", cur)));
                    if let Some(v) = view {
                        sink(EngineEvent::Tree(v));
                    }
                }
            });
            let walk = Walk { tree: &b, scope: &scope, matcher: matcher.as_ref(), sink, cancel: &cancel, stats: &stats };
            walk.run(&self.pool, vec![Job { path: scope.root.clone(), idx: 0, in_match: false }]);
            done.store(true, Ordering::Relaxed);
        });

        if cancel.load(Ordering::Relaxed) {
            *self.building.write().unwrap() = None;
            let q = self.queued.lock().unwrap().take();
            if let Some((items, _)) = q {
                if !items.is_empty() {
                    let changed = self.refresh_internal(items, matcher.as_ref(), sink);
                    if !changed.is_empty() {
                        sink(EngineEvent::Changed(changed));
                    }
                }
            }
            return Err("Scan cancelled".into());
        }

        let mut t = std::mem::take(&mut *b.write().unwrap());
        t.complete = true;
        t.clear_pending();
        let mut st = ScanStats {
            files: t.files[0] as u64,
            dirs: stats.dirs.load(Ordering::Relaxed),
            bytes: t.bytes[0],
            nodes: t.live() as u64,
            unreadable: count_unreadable(&t),
            elapsed_ms: 0,
            tree_bytes: t.heap_bytes() as u64,
            snapshot_bytes: 0,
            list_ms: stats.list_ns.load(Ordering::Relaxed) / 1_000_000,
            lock_wait_ms: stats.lock_wait_ns.load(Ordering::Relaxed) / 1_000_000,
        };
        *self.current.write().unwrap() = t;
        *self.building.write().unwrap() = None;
        self.resumable.store(true, Ordering::Relaxed);
        let q = self.queued.lock().unwrap().take();
        let mut changed = vec![scope.root_str.clone()];
        if let Some((items, max_id)) = q {
            if !items.is_empty() {
                changed.extend(self.refresh_internal(items, matcher.as_ref(), sink));
            }
            let mut cur = self.current.write().unwrap();
            cur.event_id = cur.event_id.max(max_id);
        }
        st.elapsed_ms = start.elapsed().as_millis() as u64;
        sink(EngineEvent::Progress(progress("done", String::new())));
        if let Some(v) = self.tree(None, 3) {
            sink(EngineEvent::Tree(v));
        }
        sink(EngineEvent::Volume(self.volume()));
        sink(EngineEvent::Changed(changed));
        st.snapshot_bytes = self.save_snapshot().unwrap_or(0);
        Ok(st)
    }

    // ---------------------------------------------------------------- queries

    /// `path` (None = root) with `depth` levels of children.
    pub fn tree(&self, path: Option<&str>, depth: u32) -> Option<TreeNode> {
        self.with_view(|t| {
            let i = match path {
                None => 0,
                Some(p) => t.lookup(p)?,
            };
            Some(t.view(i, depth))
        })?
    }

    /// All children of a folder (one level; only its own small files are folded).
    pub fn list(&self, path: &str) -> Option<TreeNode> {
        self.with_view(|t| t.lookup(path).map(|i| t.list(i)))?
    }

    /// Folders and kept files whose name contains `query` (ASCII case-insensitive), largest first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<TreeNode> {
        let q = query.trim().as_bytes().to_vec();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        self.with_view(|t| {
            let hits = top_k(limit, (1..t.len() as u32).filter(|&i| !t.is_dead(i)).filter(|&i| contains_ci(t.name(i).as_bytes(), &q)).map(|i| (t.bytes[i as usize], i)));
            hits.into_iter().map(|i| t.flat(i)).collect()
        })
        .unwrap_or_default()
    }

    /// Biggest kept files, optionally only below `under`.
    pub fn largest_files(&self, under: Option<&str>, limit: usize) -> Vec<TreeNode> {
        self.with_view(|t| {
            let hits = match under {
                None => top_k(limit, (1..t.len() as u32).filter(|&i| t.flags[i as usize] & (F_FILE | F_DEAD) == F_FILE).map(|i| (t.bytes[i as usize], i))),
                Some(p) => match t.lookup(p) {
                    Some(r) => top_k(limit, t.subtree(r).filter(|&i| t.flags[i as usize] & F_FILE != 0).map(|i| (t.bytes[i as usize], i))),
                    None => Vec::new(),
                },
            };
            hits.into_iter().map(|i| t.flat(i)).collect()
        })
        .unwrap_or_default()
    }

    /// One node (no children). None for paths the tree doesn't hold (incl. small files).
    pub fn size_of(&self, path: &str) -> Option<TreeNode> {
        self.with_view(|t| t.lookup(path).map(|i| t.flat(i)))?
    }

    /// Current totals of the matched folders/files at or above each path (deduplicated).
    pub fn matched_at(&self, paths: &[String]) -> Vec<Matched> {
        self.with_view(|t| {
            let mut seen = HashSet::new();
            paths.iter().filter_map(|p| t.nearest(p)).filter_map(|i| t.matched_ancestor(i)).filter(|m| seen.insert(*m)).map(|m| t.matched(m)).collect()
        })
        .unwrap_or_default()
    }

    /// Every matched folder/file at or below `path` (None = the whole tree) with current totals.
    /// Use after `load_snapshot()` to rebuild findings without rescanning.
    pub fn matched_under(&self, path: Option<&str>) -> Vec<Matched> {
        self.with_view(|t| {
            let root = match path {
                None => 0,
                Some(p) => match t.lookup(p) {
                    Some(i) => i,
                    None => return Vec::new(),
                },
            };
            let mut out: Vec<Matched> = t
                .tags
                .keys()
                .copied()
                .filter(|&i| (i as usize) < t.len() && !t.is_dead(i) && t.flags[i as usize] & F_MATCHED != 0)
                .filter(|&i| root == 0 || {
                    let mut at = i;
                    while at != NONE && at != root {
                        at = t.parent[at as usize];
                    }
                    at == root
                })
                .map(|i| t.matched(i))
                .collect();
            out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
            out
        })
        .unwrap_or_default()
    }

    /// Volume totals and where the space the walk can't see went.
    pub fn volume(&self) -> VolumeInfo {
        let scope = self.scope();
        let apfs = self.vcache.lock().unwrap().apfs(&scope.root_str);
        let (scanned, unreadable) = self.with_view(|t| (t.bytes[0], count_unreadable(t))).unwrap_or((0, 0));
        volume::info(&scope.root_str, &apfs, scanned, unreadable, scope.fda)
    }

    /// Approximate heap held by the trees.
    pub fn memory_bytes(&self) -> usize {
        let b = self.building.read().unwrap().clone().map(|b| b.read().unwrap().heap_bytes()).unwrap_or(0);
        self.current.read().unwrap().heap_bytes() + b
    }

    // ---------------------------------------------------------------- changes

    /// Paths Disko just removed: drop them and fix every ancestor's totals. Paths that still exist
    /// (a partial failure) are re-listed instead; small files the tree folded re-list their folder.
    /// Returns the deepest folders whose totals changed.
    pub fn removed(&self, paths: &[String]) -> Vec<String> {
        let mut changed = BTreeSet::new();
        let mut relist = Vec::new();
        let building = self.building.read().unwrap().clone();
        for p in paths {
            let p = p.trim_end_matches('/');
            if p.is_empty() {
                continue;
            }
            let still_there = std::fs::symlink_metadata(p).is_ok();
            {
                let mut t = self.current.write().unwrap();
                match t.lookup(p) {
                    Some(0) => {}
                    Some(i) if !still_there => {
                        let par = t.parent[i as usize];
                        t.remove(i);
                        changed.insert(t.path(par));
                    }
                    Some(_) => relist.push((p.to_string(), true)),
                    None => {
                        if let Some(parent) = Path::new(p).parent().and_then(|x| x.to_str()) {
                            if t.lookup(parent).is_some() {
                                relist.push((parent.to_string(), false));
                            }
                        }
                    }
                }
            }
            if let Some(b) = &building {
                if !still_there {
                    let mut t = b.write().unwrap();
                    if let Some(i) = t.lookup(p) {
                        if i != 0 {
                            t.remove(i);
                        }
                    }
                }
            }
        }
        if !relist.is_empty() {
            let m = self.matcher.lock().unwrap().clone();
            let noop = |_: EngineEvent| {};
            changed.extend(self.refresh_internal(relist, m.as_deref().unwrap_or(&NoMatch), &noop));
        }
        changed.into_iter().collect()
    }

    /// Re-list `dirs` (folders whose contents changed). New subfolders are walked; vanished ones
    /// dropped. Paths the tree doesn't hold refresh their nearest known ancestor.
    pub fn refresh(&self, dirs: &[String], matcher: &dyn WalkMatcher, sink: &(dyn Fn(EngineEvent) + Sync)) -> Vec<String> {
        let items: Vec<(String, bool)> = dirs.iter().map(|d| (d.clone(), false)).collect();
        if let Some((q, _)) = self.queued.lock().unwrap().as_mut() {
            q.extend(items.iter().cloned());
        }
        self.refresh_internal(items, matcher, sink)
    }

    /// Watch the root with FSEvents (replaying history since the tree's event id when possible).
    pub fn watch(self: &Arc<Self>, matcher: Arc<dyn WalkMatcher>, sink: Arc<dyn Fn(EngineEvent) + Send + Sync>) -> WatchHandle {
        *self.matcher.lock().unwrap() = Some(matcher.clone());
        let since = {
            let cur = self.current.read().unwrap();
            if !cur.is_empty() && cur.event_id > 0 && self.resumable.load(Ordering::Relaxed) {
                cur.event_id
            } else if let Some(b) = self.building.read().unwrap().clone() {
                b.read().unwrap().event_id
            } else {
                ffi::kFSEventStreamEventIdSinceNow
            }
        };
        watch::start(self, matcher, sink, since)
    }

    /// Turn a batch of FSEvents into refreshes (or queue them while a scan runs).
    pub(crate) fn apply_events(&self, batch: Vec<watch::Raw>, matcher: &dyn WalkMatcher, sink: &(dyn Fn(EngineEvent) + Sync)) -> Vec<String> {
        let scope = self.scope();
        let mut items = Vec::new();
        let mut max_id = 0u64;
        let mut rescan: Option<&str> = None;
        for ev in batch {
            if ev.flags & ffi::kFSEventStreamEventFlagHistoryDone != 0 {
                continue;
            }
            max_id = max_id.max(ev.id);
            if ev.flags & (ffi::kFSEventStreamEventFlagUserDropped | ffi::kFSEventStreamEventFlagKernelDropped) != 0 {
                rescan = Some("macOS dropped file system events");
                continue;
            }
            if ev.flags & ffi::kFSEventStreamEventFlagEventIdsWrapped != 0 {
                rescan = Some("File system event ids wrapped");
                continue;
            }
            if ev.flags & ffi::kFSEventStreamEventFlagRootChanged != 0 {
                rescan = Some("The scanned folder moved");
                continue;
            }
            let Some(p) = scope.event_path(&ev.path) else { continue };
            let deep = ev.flags & ffi::kFSEventStreamEventFlagMustScanSubDirs != 0;
            if deep && scope.is_whole(&p) {
                rescan = Some("macOS asked for a full rescan");
                continue;
            }
            items.push((p, deep));
        }
        {
            let mut q = self.queued.lock().unwrap();
            if let Some((qi, qid)) = q.as_mut() {
                qi.extend(items);
                *qid = (*qid).max(max_id);
                return Vec::new();
            }
        }
        if let Some(r) = rescan {
            self.resumable.store(false, Ordering::Relaxed);
            sink(EngineEvent::NeedsScan(r.to_string()));
        }
        let changed = if items.is_empty() { Vec::new() } else { self.refresh_internal(items, matcher, sink) };
        if max_id > 0 {
            let mut t = self.current.write().unwrap();
            t.event_id = t.event_id.max(max_id);
        }
        changed
    }

    fn refresh_internal(&self, mut items: Vec<(String, bool)>, matcher: &dyn WalkMatcher, sink: &(dyn Fn(EngineEvent) + Sync)) -> Vec<String> {
        let _g = self.refresh_lock.lock().unwrap();
        let scope = self.scope();
        // Deduplicate; a deep refresh covers everything below it.
        for it in items.iter_mut() {
            if it.0.len() > 1 {
                it.0 = it.0.trim_end_matches('/').to_string();
            }
        }
        items.sort();
        items.dedup_by(|b, a| a.0 == b.0 && {
            a.1 |= b.1;
            true
        });
        let deep: Vec<String> = items.iter().filter(|i| i.1).map(|i| format!("{}/", i.0)).collect();
        items.retain(|i| !deep.iter().any(|d| i.0.starts_with(d.as_str())));

        let mut changed = BTreeSet::new();
        let mut jobs = Vec::new();
        let mut matched_now: Vec<Matched> = Vec::new();
        let mut listing = bulk::Listing::default();
        let mut done_dirs = HashSet::new();
        let mut gone_all: Vec<String> = Vec::new();
        for (path, deep) in items {
            // Resolve to a folder the tree holds.
            let target = {
                let mut t = self.current.write().unwrap();
                if t.is_empty() {
                    break;
                }
                let mut i = match t.lookup(&path) {
                    Some(i) => i,
                    None => match t.nearest(&path) {
                        Some(a) => a,
                        None => continue,
                    },
                };
                if deep && t.lookup(&path) == Some(i) && i != 0 {
                    // Must rescan below: drop the folder; re-listing its parent walks it afresh.
                    let par = t.parent[i as usize];
                    gone_all.extend(t.remove(i));
                    i = par;
                } else if deep && i == 0 && t.lookup(&path) == Some(0) {
                    drop(t);
                    self.resumable.store(false, Ordering::Relaxed);
                    sink(EngineEvent::NeedsScan("macOS asked for a full rescan".into()));
                    continue;
                }
                while !t.is_dir(i) {
                    i = t.parent[i as usize];
                }
                if t.err[i as usize] == E_FDA && !scope.fda {
                    continue;
                }
                i
            };
            if !done_dirs.insert(target) {
                continue;
            }
            let dir_path = self.current.read().unwrap().path(target);
            let res = bulk::list(Path::new(&dir_path), &mut listing);
            let mut t = self.current.write().unwrap();
            if t.is_dead(target) {
                continue;
            }
            match res {
                Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT) | Some(libc::ENOTDIR)) => {
                    if target != 0 {
                        let par = t.parent[target as usize];
                        gone_all.extend(t.remove(target));
                        changed.insert(t.path(par));
                    }
                }
                Err(e) => {
                    let code = err_code(e.raw_os_error().unwrap_or(libc::EIO), scope.fda);
                    if t.err[target as usize] != code {
                        t.err[target as usize] = code;
                        changed.insert(dir_path);
                    }
                }
                Ok(()) => {
                    let dirty = diff(&mut t, target, &dir_path, &listing, &scope, matcher, &mut jobs, &mut matched_now, &mut gone_all);
                    if dirty {
                        changed.insert(dir_path);
                    }
                }
            }
            drop(t);
            // Walk new subfolders now, so later items of this batch (often those very folders)
            // find them complete instead of listing them a second time.
            if !jobs.is_empty() {
                if !gone_all.is_empty() {
                    sink(EngineEvent::Gone(std::mem::take(&mut gone_all)));
                }
                let stats = Stats::default();
                let never = AtomicBool::new(false);
                let walk = Walk { tree: &self.current, scope: &scope, matcher, sink, cancel: &never, stats: &stats };
                walk.run(&self.pool, std::mem::take(&mut jobs));
            }
        }
        if !gone_all.is_empty() {
            sink(EngineEvent::Gone(gone_all));
        }
        for m in matched_now {
            sink(EngineEvent::Matched(m));
        }
        // Matched folders that contain a change have new totals.
        let changed: Vec<String> = changed.into_iter().collect();
        if !changed.is_empty() {
            for m in self.matched_at(&changed) {
                sink(EngineEvent::Matched(m));
            }
        }
        let mut t = self.current.write().unwrap();
        if t.dead > 200_000 && t.dead > t.len() / 4 {
            t.compact();
        }
        changed
    }
}

/// Re-list one folder against the tree. Returns whether anything changed.
#[allow(clippy::too_many_arguments)]
fn diff(t: &mut Tree, idx: u32, dir_path: &str, l: &bulk::Listing, scope: &Scope, matcher: &dyn WalkMatcher, jobs: &mut Vec<Job>, matched: &mut Vec<Matched>, gone: &mut Vec<String>) -> bool {
    use std::collections::HashMap;
    let min = scope.min_file;
    let in_match = t.matched_ancestor(idx).is_some();
    let existing: HashMap<String, u32> = t.children(idx).map(|c| (t.name(c).to_string(), c)).collect();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut dirty = false;
    let dir = Path::new(dir_path);
    let (mut own_b, mut own_f, mut own_n) = (0u64, 0u32, 0u32);
    let mut siblings: Option<Vec<&str>> = None;
    let mut links_here = HashSet::new();
    for e in &l.ents {
        if e.err != 0 {
            continue;
        }
        let name = l.name(e);
        let have = existing.get(name).copied();
        match e.kind {
            bulk::Kind::Dir => {
                let path = dir.join(name);
                if scope_skip(scope, dir, &path, name, e) {
                    continue;
                }
                if let Some(c) = have {
                    if t.is_dir(c) {
                        seen.insert(c);
                        continue;
                    }
                    gone.extend(t.remove(c));
                }
                dirty = true;
                let c = t.add_child(idx, name, F_DIR);
                seen.insert(c);
                if scope.is_gated(&path, name) {
                    t.err[c as usize] = E_FDA;
                    continue;
                }
                let tag = if in_match {
                    None
                } else {
                    let sib = siblings.get_or_insert_with(|| l.ents.iter().map(|x| l.name(x)).collect());
                    matcher.on_dir(&path, name, sib)
                };
                if let Some(tag) = tag {
                    t.flags[c as usize] |= F_MATCHED;
                    t.tags.insert(c, tag);
                    t.cat[c as usize] = cat_code(matcher.category(tag));
                }
                t.set_pending(c, 1);
                jobs.push(Job { path, idx: c, in_match: in_match || tag.is_some() });
            }
            _ => {
                let bytes = if walk::shares(e) {
                    match walk::counted_bytes(t, e, idx, &mut links_here) {
                        Some(b) => b,
                        None => continue,
                    }
                } else {
                    e.alloc
                };
                let n = secs(e.mtime);
                if bytes >= min && e.kind == bulk::Kind::File {
                    if let Some(c) = have {
                        if t.flags[c as usize] & F_FILE != 0 {
                            seen.insert(c);
                            let cu = c as usize;
                            if t.own_bytes[cu] != bytes || t.own_newest[cu] != n {
                                dirty = true;
                                let d = bytes as i64 - t.own_bytes[cu] as i64;
                                let older = n < t.own_newest[cu];
                                t.own_bytes[cu] = bytes;
                                t.own_newest[cu] = n;
                                t.add_up(c, d, 0, n);
                                if older {
                                    t.renew_up(c);
                                }
                                if t.flags[cu] & F_MATCHED != 0 {
                                    matched.push(t.matched(c));
                                }
                            }
                            continue;
                        }
                        gone.extend(t.remove(c));
                    }
                    dirty = true;
                    let c = t.add_child(idx, name, F_FILE);
                    seen.insert(c);
                    let cu = c as usize;
                    t.own_bytes[cu] = bytes;
                    t.own_files[cu] = 1;
                    t.own_newest[cu] = n;
                    t.add_up(c, bytes as i64, 1, n);
                    if !in_match {
                        if let Some(tag) = matcher.on_file(&dir.join(name), name, bytes, e.mtime) {
                            t.flags[cu] |= F_MATCHED;
                            t.tags.insert(c, tag);
                            t.cat[cu] = cat_code(matcher.category(tag));
                            matched.push(t.matched(c));
                        }
                    }
                } else {
                    own_b += bytes;
                    own_f += 1;
                    own_n = own_n.max(n);
                }
            }
        }
    }
    let stale: Vec<u32> = existing.values().copied().filter(|c| !seen.contains(c)).collect();
    for c in stale {
        dirty = true;
        gone.extend(t.remove(c));
    }
    let u = idx as usize;
    if t.own_bytes[u] != own_b || t.own_files[u] != own_f || t.own_newest[u] != own_n {
        dirty = true;
        let older = own_n < t.own_newest[u];
        let (db, df) = (own_b as i64 - t.own_bytes[u] as i64, own_f as i64 - t.own_files[u] as i64);
        t.own_bytes[u] = own_b;
        t.own_files[u] = own_f;
        t.own_newest[u] = own_n;
        t.add_up(idx, db, df, own_n);
        if older {
            t.renew_up(idx);
        }
    }
    if t.err[u] != E_NONE {
        t.err[u] = E_NONE;
        dirty = true;
    }
    dirty
}

fn scope_skip(scope: &Scope, dir: &Path, path: &Path, name: &str, e: &bulk::Ent) -> bool {
    scope.skip_dir_pub(dir, path, name, e.dev, e.mount)
}

fn count_unreadable(t: &Tree) -> u64 {
    t.err.iter().zip(t.flags.iter()).filter(|(e, f)| **e != E_NONE && **f & F_DEAD == 0).count() as u64
}

fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len() && hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Indices of the `k` largest (bytes, idx) pairs, largest first.
fn top_k(k: usize, it: impl Iterator<Item = (u64, u32)>) -> Vec<u32> {
    use std::cmp::Reverse;
    if k == 0 {
        return Vec::new();
    }
    let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::with_capacity(k + 1);
    for x in it {
        if heap.len() < k {
            heap.push(Reverse(x));
        } else if x.0 > heap.peek().unwrap().0 .0 {
            heap.pop();
            heap.push(Reverse(x));
        }
    }
    let mut v: Vec<(u64, u32)> = heap.into_iter().map(|r| r.0).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.into_iter().map(|x| x.1).collect()
}
