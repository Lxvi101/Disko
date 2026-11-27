//! The running Disko, without any UI: one engine, one clutter set, memory, and the current
//! findings. The Tauri app and the MCP server (`Disko --mcp`) both drive a `Hub`; the app passes
//! an `emit` that forwards events to the webview.

use disko_clutter::{Clutter, PatternRule};
use disko_core::{ops, Category, CleanFailure, CleanResult, Finding, Matched, ScanProgress, Trashed, VolumeInfo, WalkMatcher};
use disko_engine::{Engine, EngineEvent, WatchHandle};
use disko_memory::{LearnedMatcher, Memory, MemoryOp, Store};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Tags at or above this come from learned/community rules; below, from the clutter crate.
const LEARNED_BASE: u32 = 1 << 30;

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// Clutter's matcher plus learned rules, swappable when memory changes.
struct Matchers {
    clutter: RwLock<Arc<Clutter>>,
    clutter_m: RwLock<Arc<dyn WalkMatcher>>,
    learned: RwLock<Arc<LearnedMatcher>>,
}

impl WalkMatcher for Matchers {
    fn on_dir(&self, path: &Path, name: &str, siblings: &[&str]) -> Option<u32> {
        if let Some(t) = self.clutter_m.read().unwrap_or_else(std::sync::PoisonError::into_inner).on_dir(path, name, siblings) {
            return Some(t);
        }
        self.learned.read().unwrap_or_else(std::sync::PoisonError::into_inner).on_dir(path, name, siblings)
    }
    fn on_file(&self, path: &Path, name: &str, bytes: u64, mtime: i64) -> Option<u32> {
        if let Some(t) = self.clutter_m.read().unwrap_or_else(std::sync::PoisonError::into_inner).on_file(path, name, bytes, mtime) {
            return Some(t);
        }
        self.learned.read().unwrap_or_else(std::sync::PoisonError::into_inner).on_file(path, name, bytes, mtime)
    }
    fn category(&self, tag: u32) -> Category {
        if tag >= LEARNED_BASE {
            self.learned.read().unwrap_or_else(std::sync::PoisonError::into_inner).category(tag)
        } else {
            self.clutter_m.read().unwrap_or_else(std::sync::PoisonError::into_inner).category(tag)
        }
    }
}

#[derive(Default)]
struct Pending {
    upsert: BTreeMap<String, Finding>,
    remove: BTreeSet<String>,
    changed: BTreeSet<String>,
}

pub struct Hub {
    pub engine: Arc<Engine>,
    matchers: Arc<Matchers>,
    store: Mutex<Store>,
    findings: RwLock<BTreeMap<String, Finding>>,
    /// Items of app storage pages (`app_storage`), so `clean` can remove them by id. Not clutter:
    /// never listed, never in routines.
    storage: RwLock<BTreeMap<String, Finding>>,
    emit: Emit,
    pending: Mutex<Pending>,
    scanning: AtomicBool,
    cancel: Mutex<Arc<AtomicBool>>,
    progress: Mutex<ScanProgress>,
    watch: Mutex<Option<WatchHandle>>,
    started: AtomicBool,
    last_done: Mutex<Option<Instant>>,
    snapshot_age: Mutex<Option<Duration>>,
    /// The MCP process watches and answers but leaves the full scan to the app when one runs.
    pub headless: bool,
}

fn learned_rules(store: &mut Store) -> Vec<disko_memory::LearnedRule> {
    store.active_rules()
}

impl Hub {
    pub fn new(emit: Emit, headless: bool) -> Arc<Hub> {
        let engine = Engine::new(disko_core::paths::cache_dir().join("snapshot.bin"));
        let mut store = Store::open_default();
        let home = disko_core::paths::home();
        let rules = learned_rules(&mut store);
        let clutter = Clutter::new(Vec::<PatternRule>::new());
        let matchers = Arc::new(Matchers {
            clutter_m: RwLock::new(clutter.matcher()),
            clutter: RwLock::new(clutter),
            learned: RwLock::new(Arc::new(LearnedMatcher::new(&rules, &home, LEARNED_BASE))),
        });
        let hub = Arc::new(Hub {
            engine,
            matchers,
            store: Mutex::new(store),
            findings: RwLock::new(BTreeMap::new()),
            storage: RwLock::new(BTreeMap::new()),
            emit,
            pending: Mutex::new(Pending::default()),
            scanning: AtomicBool::new(false),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            progress: Mutex::new(ScanProgress { phase: "none".into(), ..Default::default() }),
            watch: Mutex::new(None),
            started: AtomicBool::new(false),
            last_done: Mutex::new(None),
            snapshot_age: Mutex::new(None),
            headless,
        });
        hub.spawn_flusher();
        hub
    }

    fn clutter(&self) -> Arc<Clutter> {
        self.matchers.clutter.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    // ---------- events ----------

    fn spawn_flusher(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("disko-flush".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(200));
                let Some(hub) = weak.upgrade() else { return };
                hub.flush();
            })
            .ok();
    }

    fn flush(&self) {
        let p = std::mem::take(&mut *self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        if !p.upsert.is_empty() {
            let list: Vec<&Finding> = p.upsert.values().collect();
            (self.emit)("clutter:upsert", json!(list));
        }
        if !p.remove.is_empty() {
            (self.emit)("clutter:remove", json!(p.remove));
        }
        if !p.changed.is_empty() {
            (self.emit)("tree:changed", json!({ "paths": p.changed }));
        }
    }

    fn upsert(&self, list: Vec<Finding>) {
        if list.is_empty() {
            return;
        }
        let mut visible = Vec::new();
        let mut hidden = Vec::new();
        {
            let mut store = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let keep = store.keep();
            for f in &list {
                if keep.finding_kept_by(f).is_some() { hidden.push(f.id.clone()) } else { visible.push(f.clone()) }
            }
        }
        {
            let mut map = self.findings.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            for f in list {
                if f.paths.is_empty() || f.bytes == 0 && f.category != Category::Leftovers && f.category != Category::OldVersions {
                    map.remove(&f.id);
                    continue;
                }
                map.insert(f.id.clone(), f);
            }
        }
        let mut p = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for f in visible {
            p.remove.remove(&f.id);
            p.upsert.insert(f.id.clone(), f);
        }
        for id in hidden {
            p.upsert.remove(&id);
            p.remove.insert(id);
        }
    }

    fn drop_findings(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let mut map = self.findings.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in ids {
            map.remove(id);
            p.upsert.remove(id);
            p.remove.insert(id.clone());
        }
    }

    fn changed(&self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).changed.extend(paths);
    }

    fn on_event(&self, ev: EngineEvent) {
        match ev {
            EngineEvent::Progress(p) => {
                *self.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = p.clone();
                (self.emit)("scan:progress", json!(p));
            }
            EngineEvent::Tree(t) => (self.emit)("scan:tree", json!(t)),
            EngineEvent::Matched(m) => self.on_matched(m),
            EngineEvent::Changed(paths) => self.changed(paths),
            EngineEvent::Gone(paths) => self.on_gone(paths),
            EngineEvent::Volume(v) => (self.emit)("volume", json!(v)),
            EngineEvent::NeedsScan(_) => {
                if !self.headless || !self.engine_has_tree() {
                    self.clone_and_scan();
                }
            }
        }
    }

    fn on_matched(&self, m: Matched) {
        let f = if m.tag >= LEARNED_BASE {
            self.matchers.learned.read().unwrap_or_else(std::sync::PoisonError::into_inner).finding(&m)
        } else {
            self.clutter().on_matched(m)
        };
        if let Some(f) = f {
            self.upsert(vec![f]);
        }
    }

    /// Paths deleted outside Disko: findings lose those paths; empty ones disappear.
    fn on_gone(&self, gone: Vec<String>) {
        let gone: HashSet<String> = gone.into_iter().collect();
        let mut drop = Vec::new();
        let mut update = Vec::new();
        {
            let map = self.findings.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for f in map.values() {
                if !f.paths.iter().any(|p| gone.contains(p) || gone.iter().any(|g| p.starts_with(&format!("{}/", g)))) {
                    continue;
                }
                let left: Vec<String> = f.paths.iter().filter(|p| std::fs::symlink_metadata(p).is_ok()).cloned().collect();
                if left.is_empty() {
                    drop.push(f.id.clone());
                } else {
                    let mut g = f.clone();
                    g.bytes = left.iter().map(|p| self.engine.size_of(p).map(|n| n.bytes).unwrap_or(0)).sum();
                    g.paths = left;
                    update.push(g);
                }
            }
        }
        self.drop_findings(&drop);
        self.upsert(update);
    }

    fn engine_has_tree(&self) -> bool {
        self.engine.tree(None, 0).is_some()
    }

    // ---------- lifecycle ----------

    /// First call: show the snapshot instantly, start watching, run probes, and scan if needed.
    /// Later calls do nothing.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        // Disko 0.x kept a ~200 MB scan database per scan in ~/.disko/reports; 1.0 needs none of it.
        std::thread::spawn(|| {
            let old = disko_core::paths::home().join(".disko");
            let reports = old.join("reports");
            if reports.is_dir() {
                let _ = ops::delete(&reports);
            }
            let _ = std::fs::remove_dir(&old); // only if now empty
        });
        let age = self.engine.load_snapshot();
        *self.snapshot_age.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = age;
        if let Some(t) = self.engine.tree(None, 3) {
            (self.emit)("scan:tree", json!(t));
        }
        // Learned rules without ** are cheap to check directly.
        let hub = self.clone();
        std::thread::spawn(move || {
            let (rules, home) = {
                let mut s = hub.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                (s.active_rules(), s.home().to_path_buf())
            };
            hub.upsert(disko_memory::rules::static_findings(&rules, &home, 200));
        });
        self.start_watch();
        if self.engine.needs_scan() {
            self.clone_and_scan();
        } else {
            // Up to date from the snapshot + FSEvents replay: findings still need the probes and
            // the matched folders the snapshot already knows.
            let hub = self.clone();
            std::thread::spawn(move || hub.run_probes());
            let hub = self.clone();
            std::thread::spawn(move || hub.rematch_from_snapshot());
        }
        let hub = self.clone();
        std::thread::spawn(move || hub.background_loop());
    }

    fn start_watch(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let sink: Arc<dyn Fn(EngineEvent) + Send + Sync> = Arc::new(move |ev| {
            if let Some(h) = weak.upgrade() {
                h.on_event(ev);
            }
        });
        let m: Arc<dyn WalkMatcher> = self.matchers.clone();
        *self.watch.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(self.engine.watch(m, sink));
    }

    /// After loading a snapshot without rescanning, ask the engine for the matched folders' totals.
    fn rematch_from_snapshot(&self) {
        for m in self.engine.matched_under(None) {
            self.on_matched(m);
        }
    }

    fn run_probes(&self) {
        let clutter = self.clutter();
        clutter.probes(&|list| self.upsert(list));
    }

    fn clone_and_scan(&self) {
        // `on_event` only has &self; scanning needs an Arc. Every Hub lives in an Arc, so find it
        // through the watch sink's owner: simplest is a global registry.
        if let Some(h) = HUB.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|w| w.upgrade()) {
            h.rescan();
        }
    }

    /// Start (or restart) the one scan in the background.
    pub fn rescan(self: &Arc<Self>) {
        let cancel = {
            let mut c = self.cancel.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            c.store(true, Ordering::SeqCst);
            let fresh = Arc::new(AtomicBool::new(false));
            *c = fresh.clone();
            fresh
        };
        let hub = self.clone();
        std::thread::Builder::new()
            .name("disko-scan".into())
            .spawn(move || {
                // Wait for a previous scan to notice its cancel flag.
                while hub.scanning.swap(true, Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(50));
                    if cancel.load(Ordering::SeqCst) {
                        return;
                    }
                }
                hub.clutter().reset();
                (hub.emit)("scan:progress", json!(ScanProgress { phase: "probes".into(), ..Default::default() }));
                let probes = {
                    let h = hub.clone();
                    std::thread::spawn(move || h.run_probes())
                };
                let weak = Arc::downgrade(&hub);
                let sink = move |ev: EngineEvent| {
                    if let Some(h) = weak.upgrade() {
                        h.on_event(ev);
                    }
                };
                let m: Arc<dyn WalkMatcher> = hub.matchers.clone();
                let res = hub.engine.scan(m, cancel.clone(), &sink);
                let _ = probes.join();
                hub.scanning.store(false, Ordering::SeqCst);
                if res.is_ok() && !cancel.load(Ordering::SeqCst) {
                    *hub.last_done.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
                    *hub.snapshot_age.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Duration::ZERO);
                    let mut p = hub.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                    p.phase = "done".into();
                    p.fraction = 1.0;
                    *hub.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = p.clone();
                    (hub.emit)("scan:progress", json!(p));
                    (hub.emit)("volume", json!(hub.engine.volume()));
                }
                if let Err(e) = res {
                    eprintln!("disko: scan failed: {e}");
                }
            })
            .ok();
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning.load(Ordering::SeqCst)
    }

    pub fn progress(&self) -> ScanProgress {
        self.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn snapshot_age(&self) -> Option<Duration> {
        if let Some(t) = *self.last_done.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            return Some(t.elapsed());
        }
        *self.snapshot_age.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every few seconds: pick up memory changes made by agents; every 10 minutes: due routines.
    fn background_loop(self: Arc<Self>) {
        let mut last_routines = Instant::now() - Duration::from_secs(3600);
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let changed = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).refresh();
            if changed {
                self.memory_changed();
            }
            if !self.headless && !self.is_scanning() && last_routines.elapsed() > Duration::from_secs(600) {
                last_routines = Instant::now();
                self.run_due_routines();
            }
        }
    }

    // ---------- queries ----------

    pub fn visible_findings(&self) -> Vec<Finding> {
        let all: Vec<Finding> = self.findings.read().unwrap_or_else(std::sync::PoisonError::into_inner).values().cloned().collect();
        let mut store = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let keep = store.keep();
        let mut v: Vec<Finding> = all.into_iter().filter(|f| keep.finding_kept_by(f).is_none()).collect();
        v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        v
    }

    pub fn all_findings(&self) -> Vec<Finding> {
        self.findings.read().unwrap_or_else(std::sync::PoisonError::into_inner).values().cloned().collect()
    }

    pub fn volume(&self) -> VolumeInfo {
        self.engine.volume()
    }

    /// One app's storage page. Its items become cleanable by id; kept ones are marked busy.
    pub fn app_storage(&self, path: &str) -> Option<disko_apps::AppStorage> {
        let mut st = disko_apps::storage(Path::new(path))?;
        {
            let mut store = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let keep = store.keep();
            for f in st.groups.iter_mut().flat_map(|g| g.items.iter_mut()) {
                if keep.finding_kept_by(f).is_some() {
                    f.busy = Some("On your keep list".into());
                }
            }
        }
        let mut map = self.storage.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        for f in st.groups.iter().flat_map(|g| g.items.iter()) {
            map.insert(f.id.clone(), f.clone());
        }
        Some(st)
    }

    // ---------- cleaning ----------

    /// Clean findings by id (each with its own action, or the Trash when `trash_ids`), and plain
    /// paths (always to the Trash). The tree and findings update before this returns.
    pub fn clean(&self, ids: &[String], trash_ids: &[String], paths: &[String]) -> CleanResult {
        let storage_ids: HashSet<String>;
        let mut findings: Vec<Finding> = {
            let map = self.findings.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let smap = self.storage.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut v: Vec<Finding> = Vec::new();
            for id in ids.iter().chain(trash_ids.iter()) {
                if v.iter().any(|f| &f.id == id) {
                    continue;
                }
                if let Some(f) = map.get(id).or_else(|| smap.get(id)) {
                    v.push(f.clone());
                }
            }
            storage_ids = v.iter().filter(|f| smap.contains_key(&f.id)).map(|f| f.id.clone()).collect();
            // A tool command (e.g. `brew cleanup`) removes everything it owns, so every finding
            // sharing it joins the request: its keep/busy checks apply and it's reported honestly.
            let cmds: Vec<Vec<String>> = v
                .iter()
                .filter_map(|f| match &f.action {
                    disko_core::Action::Command { argv, .. } if !trash_ids.contains(&f.id) => Some(argv.clone()),
                    _ => None,
                })
                .collect();
            if !cmds.is_empty() {
                for f in map.values().chain(smap.values()) {
                    if v.iter().any(|x| x.id == f.id) {
                        continue;
                    }
                    if let disko_core::Action::Command { argv, .. } = &f.action {
                        if cmds.contains(argv) {
                            v.push(f.clone());
                        }
                    }
                }
            }
            v
        };
        // Storage items: what only the app's profile knows (a simulator booted meanwhile, an
        // Adobe app opened, a CLI version now in use) keeps them, like busy findings.
        let mut storage_skipped: Vec<CleanFailure> = Vec::new();
        let storage_items: Vec<Finding> = findings.iter().filter(|f| storage_ids.contains(&f.id)).cloned().collect();
        if !storage_items.is_empty() {
            let blocked = disko_apps::storage::recheck(&storage_items);
            findings.retain(|f| {
                let Some((_, why)) = blocked.iter().find(|(id, _)| id == &f.id) else { return true };
                if f.paths.is_empty() {
                    storage_skipped.push(CleanFailure { path: f.title.clone(), error: why.clone() });
                }
                for p in &f.paths {
                    storage_skipped.push(CleanFailure { path: p.clone(), error: why.clone() });
                }
                false
            });
        }
        for f in findings.iter_mut() {
            if trash_ids.contains(&f.id) && f.action != disko_core::Action::Trash {
                f.action = disko_core::Action::Trash;
                f.safety = disko_core::Safety::Review;
            }
        }
        let is_kept = |p: &str| self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_kept(p);
        let mut result = if findings.is_empty() {
            CleanResult::default()
        } else {
            self.clutter().clean(&findings, &is_kept, &mut |_, _, _| {})
        };
        result.skipped.extend(storage_skipped);
        for p in paths {
            if is_kept(p) {
                result.skipped.push(CleanFailure { path: p.clone(), error: "On your keep list.".into() });
                continue;
            }
            if let Err(e) = disko_clutter::clean::check_path(Path::new(p)) {
                result.failed.push(CleanFailure { path: p.clone(), error: e });
                continue;
            }
            let bytes = self.engine.size_of(p).map(|n| n.bytes).unwrap_or_else(|| ops::allocated(Path::new(p)));
            match ops::trash(Path::new(p)) {
                Ok(to) => {
                    // Moving to the Trash frees nothing yet: bytes are in `trashed`, not `freed`.
                    result.trashed.push(Trashed { from: p.clone(), to: to.to_string_lossy().into(), bytes });
                }
                Err(e) => result.failed.push(CleanFailure { path: p.clone(), error: e }),
            }
        }
        {
            let mut smap = self.storage.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            for id in &result.removed {
                smap.remove(id);
            }
        }
        if !result.removed.is_empty() {
            disko_apps::storage::invalidate();
            // A clutter finding whose tool command names something just removed from an app page
            // (`simctl delete <udid> …`) would fail now; drop it until the probes see it again.
            let gone_args: HashSet<String> = findings
                .iter()
                .filter(|f| storage_ids.contains(&f.id) && result.removed.contains(&f.id))
                .filter_map(|f| match &f.action {
                    disko_core::Action::Command { argv, .. } => argv.last().cloned(),
                    _ => None,
                })
                .collect();
            if !gone_args.is_empty() {
                let stale: Vec<String> = self
                    .findings
                    .read()
                    .unwrap()
                    .values()
                    .filter(|f| matches!(&f.action, disko_core::Action::Command { argv, .. } if argv.iter().any(|a| gone_args.contains(a))))
                    .map(|f| f.id.clone())
                    .collect();
                self.drop_findings(&stale);
            }
        }
        self.after_removal(&findings, &result);
        disko_apps::size::invalidate(None);
        result
    }

    /// Reflect a removal everywhere, immediately.
    fn after_removal(&self, findings: &[Finding], result: &CleanResult) {
        let mut gone: Vec<String> = result.trashed.iter().map(|t| t.from.clone()).collect();
        gone.extend(result.deleted.iter().cloned());
        for f in findings.iter().filter(|f| result.removed.contains(&f.id)) {
            gone.extend(f.paths.iter().filter(|p| std::fs::symlink_metadata(p).is_err()).cloned());
        }
        gone.sort();
        gone.dedup();
        let changed = self.engine.removed(&gone);
        self.changed(changed);
        self.drop_findings(&result.removed);
        self.on_gone(gone);
        // Things moved to the Trash show up there right away.
        let trash = disko_core::paths::home().join(".Trash").to_string_lossy().to_string();
        if !result.trashed.is_empty() {
            let m = self.matchers.clone();
            let changed = self.engine.refresh(&[trash], &*m, &|ev| self.on_event(ev));
            self.changed(changed);
        }
        self.flush();
        (self.emit)("volume", json!(self.engine.volume()));
    }

    pub fn undo(&self, trashed: &[Trashed]) -> (usize, Vec<CleanFailure>) {
        let mut ok = 0;
        let mut failed = Vec::new();
        let mut dirs = BTreeSet::new();
        for t in trashed {
            match ops::put_back(Path::new(&t.to), Path::new(&t.from)) {
                Ok(()) => {
                    ok += 1;
                    if let Some(parent) = Path::new(&t.from).parent() {
                        dirs.insert(parent.to_string_lossy().to_string());
                    }
                }
                Err(e) => failed.push(CleanFailure { path: t.from.clone(), error: e }),
            }
        }
        dirs.insert(disko_core::paths::home().join(".Trash").to_string_lossy().to_string());
        let dirs: Vec<String> = dirs.into_iter().collect();
        let m = self.matchers.clone();
        let changed = self.engine.refresh(&dirs, &*m, &|ev| self.on_event(ev));
        self.changed(changed);
        self.flush();
        (self.emit)("volume", json!(self.engine.volume()));
        (ok, failed)
    }

    pub fn uninstall(&self, paths: &[String]) -> CleanResult {
        let u = disko_apps::uninstall(paths);
        let mut r = u.result;
        r.deleted.extend(u.deleted);
        r.notes.extend(u.notes);
        self.after_removal(&[], &r);
        r
    }

    // ---------- memory & routines ----------

    pub fn memory(&self) -> Memory {
        self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).memory()
    }

    pub fn memory_update(&self, op: MemoryOp) -> Result<Memory, String> {
        // Changes made in the app are the user's own.
        let op = match op {
            MemoryOp::SetRoutine { mut routine } => {
                routine.by_agent = false;
                MemoryOp::SetRoutine { routine }
            }
            other => other,
        };
        let m = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).apply(op)?;
        self.memory_changed();
        Ok(m)
    }

    /// Keep list or rules changed (here or in another process): re-filter and re-match.
    pub fn memory_changed(&self) {
        let (rules, home, mem) = {
            let mut s = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (s.active_rules(), s.home().to_path_buf(), s.memory())
        };
        *self.matchers.learned.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(LearnedMatcher::new(&rules, &home, LEARNED_BASE));
        // Drop findings of rules that no longer exist, add static matches of current ones.
        let ids: Vec<String> = {
            let map = self.findings.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            map.values().filter(|f| f.rule.starts_with("learned.") || f.rule.starts_with("community.")).map(|f| f.id.clone()).collect()
        };
        self.drop_findings(&ids);
        self.upsert(disko_memory::rules::static_findings(&rules, &home, 200));
        // Re-announce everything so kept items disappear and un-kept ones come back.
        let all = self.all_findings();
        self.upsert(all);
        self.flush();
        (self.emit)("memory", json!(mem));
    }

    pub fn run_routine(&self, id: &str) -> Result<CleanResult, String> {
        let mem = self.memory();
        let routine = mem.routines.iter().find(|r| r.id == id).cloned().ok_or("No such routine.")?;
        Ok(self.run_one_routine(&routine))
    }

    fn run_one_routine(&self, routine: &disko_memory::Routine) -> CleanResult {
        let now = disko_memory::now();
        let all = self.all_findings();
        let picked = {
            let mut s = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            disko_memory::select(&all, routine, now, s.keep())
        };
        // Agent-made routines only move things to the Trash (tool commands still run as such).
        let (ids, trash): (Vec<String>, Vec<String>) = if routine.by_agent {
            let (cmd, rest): (Vec<&Finding>, Vec<&Finding>) = picked.iter().partition(|f| matches!(f.action, disko_core::Action::Command { .. }));
            (cmd.iter().map(|f| f.id.clone()).collect(), rest.iter().map(|f| f.id.clone()).collect())
        } else {
            (picked.iter().map(|f| f.id.clone()).collect(), Vec::new())
        };
        let r = self.clean(&ids, &trash, &[]);
        let _ = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).record_run(&routine.id, now, r.freed);
        (self.emit)("memory", json!(self.memory()));
        r
    }

    fn run_due_routines(&self) {
        // Only with a complete picture: after a scan finished or a fresh snapshot.
        if self.snapshot_age().is_none() {
            return;
        }
        let due = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).due_routines(disko_memory::now());
        for r in due {
            let res = self.run_one_routine(&r);
            if res.freed > 0 {
                (self.emit)("routine:ran", json!({ "id": r.id, "title": r.title, "freed": res.freed }));
            }
        }
    }
}

/// The one Hub of this process (for code paths that only have `&Hub`).
pub static HUB: Mutex<Option<std::sync::Weak<Hub>>> = Mutex::new(None);

pub fn create(emit: Emit, headless: bool) -> Arc<Hub> {
    let hub = Hub::new(emit, headless);
    *HUB.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::downgrade(&hub));
    hub
}
