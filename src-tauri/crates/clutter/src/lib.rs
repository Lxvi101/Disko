//! # disko-clutter
//!
//! Everything that decides what is clutter on a Mac and how to remove it. The yardstick is the
//! product principle: *how did this Mac look when it left the factory, and how close can we get
//! back?* Clutter is what accumulated through use — build output, caches, package stores, old
//! versions beside the one in use, leftovers of removed apps, installers that already did their
//! job — never the owner's current work, chosen tools or personal files (those are only ever
//! *suggested* as Review, and only with evidence).
//!
//! ## API
//!
//! ```ignore
//! let clutter = Clutter::new(extra_rules);          // Arc<Clutter>
//! engine.walk(clutter.matcher());                     // Arc<dyn WalkMatcher>, called per dir/file
//! // for every `Matched` the engine reports once a tagged subtree is measured:
//! if let Some(f) = clutter.on_matched(m) { upsert(f) } // may be an updated *group* finding (same id)
//! // at scan start, in parallel with the walk (streams within seconds):
//! clutter.probes(&|findings| upsert_all(findings));
//! // when the user taps clean:
//! let result = clutter.clean(&findings, &|path| keep.contains(path), &mut |done, total, title| …);
//! ```
//!
//! * **Rules** ([`rules`]): a data table matched during the engine's one disk walk — build output
//!   next to its manifest (one finding per project), Electron/Chromium and sandboxed app caches
//!   (one per app), installers, core dumps, big forgotten files — plus runtime [`PatternRule`]s
//!   (user, agent, community) as globs, no code change needed.
//! * **Probes** ([`probes`]): targeted deterministic checks of known locations that run at scan
//!   start without a walk: package stores (preferring the owning tool's command), Xcode, simulators,
//!   Docker, old versions (Node, Python, Rust toolchains, rust-docs, JetBrains, agent CLIs, editor
//!   extensions, launcher-pointed releases), leftovers of uninstalled apps, dead launch agents,
//!   staged updates, verified retained ZIPs, Adobe payloads, Trash, logs.
//! * **Cleaning** ([`clean`]): executes each finding's `Action` through `disko_core::ops` only,
//!   rechecking keep and busy at execution time and never following symlinks.
//!
//! IDs are stable across scans: group findings use `finding_id(rule, [group key])`.

pub mod apps;
pub mod clean;
pub mod finding;
pub mod group;
pub mod probes;
pub mod procs;
pub mod rules;
pub mod util;
pub mod walk;

pub use clean::CleanProgress;
pub use rules::PatternRule;

use apps::Apps;
use disko_core::{Category, Finding, Matched, WalkMatcher};
use procs::Procs;
use rules::{file_ext_matches, in_scope, name_matches, Glob, Scope, APPLE_CONTAINER_CACHES, DIR_RULES, FILE_RULES, FORGOTTEN_MIN_BYTES, FORGOTTEN_MIN_DAYS};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use util::{exists, now, Size, DAY};

/// Where things are. `root` is "/" on a real Mac; tests point both at temp folders.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    pub root: PathBuf,
    /// Run external tools (ps, brew, xcrun, docker …). Off in fixture tests.
    pub tools: bool,
    /// Full Disk Access granted. Without it probes stay out of folders macOS guards with a
    /// permission prompt (Desktop, Documents, Downloads, Music, Movies, other apps' containers),
    /// so a scan never pops a dialog.
    pub fda: bool,
}

impl Env {
    pub fn system() -> Env {
        Env { home: disko_core::paths::home(), root: PathBuf::from("/"), tools: true, fda: has_full_disk_access() }
    }
    /// A fixture environment: no external tools, full access.
    pub fn fixture(home: PathBuf, root: PathBuf) -> Env {
        Env { home, root, tools: false, fda: true }
    }
    /// May probes read this path without risking a permission prompt?
    pub fn readable(&self, path: &Path) -> bool {
        if self.fda {
            return true;
        }
        let Ok(rel) = path.strip_prefix(&self.home) else { return true };
        !["Desktop", "Documents", "Downloads", "Music", "Movies", "Pictures", "Library/Containers", "Library/Group Containers", "Library/Mobile Documents", "Library/Mail", "Library/Messages", "Library/Safari", "Library/Application Support/MobileSync"]
            .iter()
            .any(|x| rel.starts_with(x))
    }
    /// A system path ("/Library/Updates") under `root`.
    pub fn sys(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }
}

/// Full Disk Access: only then can a process read the TCC database.
pub fn has_full_disk_access() -> bool {
    std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

#[derive(Debug, Clone)]
enum Tag {
    Dir(usize),
    File(usize),
    /// A location a probe owns. Coloured on the map; the probe makes the finding.
    Claim(Category),
    Pattern(usize),
}

type ChildFilter = fn(&str) -> bool;

pub struct Clutter {
    pub env: Env,
    patterns: Vec<(PatternRule, Glob)>,
    tags: Vec<Tag>,
    by_name: HashMap<&'static str, Vec<u32>>,
    wild_names: Vec<(&'static str, u32)>,
    file_tags: Vec<u32>,
    claims_exact: HashMap<PathBuf, u32>,
    claims_children: HashMap<PathBuf, (u32, ChildFilter)>,
    patterns_by_depth: HashMap<usize, Vec<u32>>,
    pub(crate) groups: Mutex<HashMap<String, group::Group>>,
    procs: Mutex<Option<(Instant, Arc<Procs>)>>,
    pinned_procs: Option<Arc<Procs>>,
    apps: OnceLock<Arc<Apps>>,
    pub(crate) git_roots: Mutex<HashMap<PathBuf, PathBuf>>,
}

impl Clutter {
    /// Rules for this Mac plus `extra` runtime rules (user, agent, community).
    pub fn new(extra: Vec<PatternRule>) -> Arc<Clutter> {
        Arc::new(Clutter::build(Env::system(), extra, None, None))
    }

    /// For tests and tools: a custom environment, optionally with a fixed process snapshot and
    /// app inventory (so busy/ownership checks are deterministic).
    pub fn with_env(env: Env, extra: Vec<PatternRule>, procs: Option<Procs>, apps: Option<Apps>) -> Arc<Clutter> {
        Arc::new(Clutter::build(env, extra, procs, apps))
    }

    fn build(env: Env, extra: Vec<PatternRule>, procs: Option<Procs>, apps_fixed: Option<Apps>) -> Clutter {
        let mut tags = vec![];
        let mut by_name: HashMap<&'static str, Vec<u32>> = HashMap::new();
        let mut wild_names = vec![];
        for (i, r) in DIR_RULES.iter().enumerate() {
            let t = tags.len() as u32;
            tags.push(Tag::Dir(i));
            for n in r.names {
                if n.starts_with('*') || n.ends_with('*') {
                    wild_names.push((*n, t));
                } else {
                    by_name.entry(*n).or_default().push(t);
                }
            }
        }
        let mut file_tags = vec![];
        for i in 0..FILE_RULES.len() {
            file_tags.push(tags.len() as u32);
            tags.push(Tag::File(i));
        }
        let mut claims_exact = HashMap::new();
        for (rel, cat) in probes::CLAIMS {
            let p = if rel.starts_with('/') { env.sys(rel) } else { env.home.join(rel) };
            claims_exact.insert(p, tags.len() as u32);
            tags.push(Tag::Claim(*cat));
        }
        let mut claims_children = HashMap::new();
        for (rel, cat, filter) in probes::CLAIM_CHILDREN {
            let p = if rel.starts_with('/') { env.sys(rel) } else { env.home.join(rel) };
            claims_children.insert(p, (tags.len() as u32, *filter));
            tags.push(Tag::Claim(*cat));
        }
        let mut patterns = vec![];
        let mut patterns_by_depth: HashMap<usize, Vec<u32>> = HashMap::new();
        for r in extra {
            if let Some(g) = Glob::new(&r.pattern, &env.home) {
                patterns_by_depth.entry(g.depth()).or_default().push(tags.len() as u32);
                tags.push(Tag::Pattern(patterns.len()));
                patterns.push((r, g));
            }
        }
        let apps = OnceLock::new();
        if let Some(a) = apps_fixed {
            let _ = apps.set(Arc::new(a));
        }
        Clutter {
            env,
            patterns,
            tags,
            by_name,
            wild_names,
            file_tags,
            claims_exact,
            claims_children,
            patterns_by_depth,
            groups: Mutex::new(HashMap::new()),
            procs: Mutex::new(None),
            pinned_procs: procs.map(Arc::new),
            apps,
            git_roots: Mutex::new(HashMap::new()),
        }
    }

    /// The hook the engine calls during its walk.
    pub fn matcher(self: &Arc<Self>) -> Arc<dyn WalkMatcher> {
        self.clone()
    }

    /// Forget grouped walk state (call before a rescan).
    pub fn reset(&self) {
        self.groups.lock().unwrap().clear();
        self.git_roots.lock().unwrap().clear();
        *self.procs.lock().unwrap() = None;
    }

    /// A tagged subtree is measured. Returns the (possibly updated group) finding, if any.
    pub fn on_matched(&self, m: Matched) -> Option<Finding> {
        match self.tags.get(m.tag as usize)? {
            Tag::Dir(i) => self.dir_match(*i, &DIR_RULES[*i], &m),
            Tag::File(i) => self.file_match(*i, &m),
            Tag::Claim(_) => None,
            Tag::Pattern(i) => {
                let (r, _) = &self.patterns[*i];
                Some(self.pattern_match(r, &m.path, Size { bytes: m.bytes, files: m.files, newest: m.newest }))
            }
        }
    }

    /// Run every probe in parallel. `sink` receives each probe's findings as soon as it is done.
    pub fn probes(&self, sink: &(dyn Fn(Vec<Finding>) + Sync)) {
        probes::run_all(self, sink);
    }

    /// Current process snapshot (refreshed at most every 5 s).
    pub fn procs(&self) -> Arc<Procs> {
        if let Some(p) = &self.pinned_procs {
            return p.clone();
        }
        let mut g = self.procs.lock().unwrap();
        if let Some((t, p)) = g.as_ref() {
            if t.elapsed() < Duration::from_secs(5) {
                return p.clone();
            }
        }
        let p = Arc::new(if self.env.tools { Procs::snapshot() } else { Procs { list: vec![], complete: true } });
        *g = Some((Instant::now(), p.clone()));
        p
    }

    /// A fresh snapshot, for recheck right before cleaning.
    pub fn fresh_procs(&self) -> Arc<Procs> {
        if self.pinned_procs.is_none() {
            *self.procs.lock().unwrap() = None;
        }
        self.procs()
    }

    pub fn apps(&self) -> Arc<Apps> {
        self.apps.get_or_init(|| Arc::new(Apps::scan(&self.env.root, &self.env.home))).clone()
    }

    pub(crate) fn pattern_rules(&self) -> impl Iterator<Item = &(PatternRule, Glob)> {
        self.patterns.iter()
    }

    fn pattern_tag(&self, path: &Path) -> Option<u32> {
        if self.patterns.is_empty() {
            return None;
        }
        let depth = path.components().count().saturating_sub(1);
        self.patterns_by_depth.get(&depth)?.iter().copied().find(|t| matches!(&self.tags[*t as usize], Tag::Pattern(i) if self.patterns[*i].1.matches(path)))
    }

    /// Personal-file areas: home folders that aren't Library, Applications or hidden tool dirs.
    fn user_area(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.env.home) else { return false };
        let mut comps = rel.components();
        let Some(first) = comps.next() else { return false };
        let first = first.as_os_str().to_string_lossy();
        if first.starts_with('.') || first == "Library" || first == "Applications" {
            return false;
        }
        let parent = rel.parent().unwrap_or(rel);
        !parent.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            c == ".git"
                || c == "node_modules"
                || [".app", ".photoslibrary", ".fcpbundle", ".imovielibrary", ".musiclibrary", ".tvlibrary", ".logicx", ".band", ".utm", ".pvm", ".vmwarevm", ".sparsebundle", ".bundle", ".framework", ".xcarchive", ".lrdata", ".lrcat-data", ".cocatalog"]
                    .iter()
                    .any(|e| c.ends_with(e))
        })
    }
}

impl WalkMatcher for Clutter {
    fn on_dir(&self, path: &Path, name: &str, siblings: &[&str]) -> Option<u32> {
        if let Some(t) = self.claims_exact.get(path) {
            return Some(*t);
        }
        if let Some(parent) = path.parent() {
            if let Some((t, f)) = self.claims_children.get(parent) {
                if f(name) {
                    return Some(*t);
                }
            }
        }
        if let Some(t) = self.pattern_tag(path) {
            return Some(t);
        }
        let exact = self.by_name.get(name).into_iter().flatten().copied();
        let wild = self.wild_names.iter().filter(|(n, _)| name_matches(n, name)).map(|(_, t)| *t);
        for t in exact.chain(wild) {
            let Tag::Dir(i) = self.tags[t as usize] else { continue };
            let r = &DIR_RULES[i];
            if !in_scope(r.scope, path, &self.env.home) {
                continue;
            }
            if !r.markers.is_empty() && !siblings.iter().any(|s| r.markers.iter().any(|m| name_matches(m, s))) {
                continue;
            }
            if !r.inside.is_empty() && !r.inside.iter().any(|f| exists(&path.join(f))) {
                continue;
            }
            if r.scope == Scope::Containers {
                let id = path.strip_prefix(self.env.home.join("Library")).ok().and_then(|p| p.components().nth(1)).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
                let id = id.trim_start_matches("group.");
                if id.starts_with("com.apple.") && !APPLE_CONTAINER_CACHES.iter().any(|a| id.starts_with(a)) {
                    continue;
                }
            }
            return Some(t);
        }
        None
    }

    fn on_file(&self, path: &Path, name: &str, bytes: u64, mtime: i64) -> Option<u32> {
        if let Some(t) = self.pattern_tag(path) {
            return Some(t);
        }
        for (i, r) in FILE_RULES.iter().enumerate() {
            let hit = match r.id {
                "installers.file" => file_ext_matches(r, name) && (self.user_area(path) || path.starts_with(self.env.home.join("Library/iTunes"))),
                "installers.partial" => {
                    file_ext_matches(r, name) && (path.starts_with(self.env.home.join("Downloads")) || path.starts_with(self.env.home.join("Desktop"))) && now() - mtime > 2 * DAY
                }
                "logs.core-dump" => file_ext_matches(r, name) && path.parent() == Some(self.env.sys("/cores").as_path()),
                _ => bytes >= FORGOTTEN_MIN_BYTES && now() - mtime > FORGOTTEN_MIN_DAYS * DAY && self.user_area(path),
            };
            if hit {
                return Some(self.file_tags[i]);
            }
        }
        None
    }

    fn category(&self, tag: u32) -> Category {
        match self.tags.get(tag as usize) {
            Some(Tag::Dir(i)) => DIR_RULES[*i].category,
            Some(Tag::File(i)) => FILE_RULES[*i].category,
            Some(Tag::Claim(c)) => *c,
            Some(Tag::Pattern(i)) => self.patterns[*i].0.category,
            None => Category::Cache,
        }
    }
}
