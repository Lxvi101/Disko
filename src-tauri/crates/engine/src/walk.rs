//! What the scan covers (devices, firmlink twins, cloud folders, privacy-gated folders) and the
//! parallel walk that fills a [`Tree`].

use crate::bulk::{self, Kind, Listing};
use crate::tree::*;
use crate::EngineEvent;
use disko_core::{Matched, WalkMatcher};
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::RwLock;

/// Folders guarded by macOS privacy protections (relative to a home folder). Without Full Disk
/// Access they are not listed (no prompts) and show up as "Needs Full Disk Access".
const GATED: &[&str] = &[
    "Library/Application Support/AddressBook",
    "Library/Application Support/CallHistoryDB",
    "Library/Application Support/CallHistoryTransactions",
    "Library/Application Support/com.apple.TCC",
    "Library/Application Support/com.apple.sharedfilelist",
    "Library/Application Support/Knowledge",
    "Library/Application Support/FileProvider",
    "Library/Application Support/MobileSync",
    "Library/Accounts",
    "Library/Assistant",
    "Library/Autosave Information",
    "Library/Biome",
    "Library/Calendars",
    "Library/Cookies",
    "Library/CoreFollowUp",
    "Library/Daemon Containers",
    "Library/DuetExpertCenter",
    "Library/HomeKit",
    "Library/IdentityServices",
    "Library/Mail",
    "Library/Messages",
    "Library/Metadata/CoreSpotlight",
    "Library/PersonalizationPortrait",
    "Library/Reminders",
    "Library/Safari",
    "Library/Sharing",
    "Library/Suggestions",
    "Library/Trial",
    ".Trash",
];

/// Folders macOS asks about once each (and remembers) when listed without Full Disk Access.
/// Walked by default (`EngineConfig::user_folders`): most projects and their build output live here.
const USER_FOLDERS: &[&str] = &["Desktop", "Documents", "Downloads"];

/// Other apps' data: listing it without Full Disk Access raises macOS 14's "data from other apps"
/// prompt. Skipped unless `EngineConfig::allow_prompts`.
const APP_DATA: &[&str] = &["Library/Containers", "Library/Group Containers"];

pub struct Scope {
    pub root: PathBuf,
    pub root_str: String,
    pub devices: Vec<u64>,
    pub excluded: HashSet<PathBuf>,
    /// Gated relative paths (empty with Full Disk Access).
    gated: Vec<&'static str>,
    gated_names: HashSet<&'static str>,
    pub fda: bool,
    pub min_file: u64,
    /// Path prefixes the watcher ignores (outside the scan, Disko's own cache).
    pub ignore: Vec<String>,
    /// Firmlinks: (data-volume twin, visible path), for translating FSEvents paths.
    pub firmlinks: Vec<(String, String)>,
}

impl Scope {
    pub fn new(cfg: &crate::EngineConfig, fda: bool, snapshot_path: &Path) -> Scope {
        let root = std::fs::canonicalize(&cfg.root).unwrap_or_else(|_| cfg.root.clone());
        let whole = root == Path::new("/");
        let mut devices = Vec::new();
        if let Ok(m) = std::fs::symlink_metadata(&root) {
            devices.push(m.dev() as u64);
        }
        let mut excluded: HashSet<PathBuf> = HashSet::new();
        let mut firmlinks = Vec::new();
        if whole {
            if let Ok(m) = std::fs::symlink_metadata("/System/Volumes/Data") {
                devices.push(m.dev() as u64);
            }
            // Firmlinks expose the data volume under /Users, /Applications …; skip their twins
            // inside /System/Volumes/Data so nothing counts twice. Data-only folders still count.
            if let Ok(list) = std::fs::read_to_string("/usr/share/firmlinks") {
                for line in list.lines() {
                    if let Some((visible, target)) = line.split_once('\t') {
                        let twin = format!("/System/Volumes/Data/{}", target.trim());
                        excluded.insert(PathBuf::from(&twin));
                        firmlinks.push((twin, visible.trim().to_string()));
                    }
                }
            }
            for p in ["/Volumes", "/dev", "/System/Volumes/VM", "/System/Volumes/Preboot", "/System/Volumes/Update", "/System/Volumes/Hardware", "/System/Volumes/xarts", "/System/Volumes/iSCPreboot"] {
                excluded.insert(PathBuf::from(p));
            }
        }
        excluded.extend(cfg.excludes.iter().cloned());
        let mut gated: Vec<&'static str> = Vec::new();
        if !fda {
            gated.extend_from_slice(GATED);
            if !cfg.user_folders {
                gated.extend_from_slice(USER_FOLDERS);
            }
            if !cfg.allow_prompts {
                gated.extend_from_slice(APP_DATA);
            }
        }
        let gated_names = gated.iter().map(|g| g.rsplit('/').next().unwrap()).collect();
        let root_str = root.to_string_lossy().to_string();
        let mut ignore = vec![];
        if whole {
            ignore.extend(["/Volumes/", "/dev/", "/System/Volumes/VM/", "/System/Volumes/Preboot/", "/System/Volumes/Update/", "/System/Volumes/Hardware/", "/System/Volumes/xarts/", "/System/Volumes/iSCPreboot/", "/.vol/", "/private/var/db/fseventsd/"].map(String::from));
        }
        if let Some(d) = snapshot_path.parent() {
            ignore.push(format!("{}/", d.to_string_lossy().trim_end_matches('/')));
        }
        Scope { root, root_str, devices, excluded, gated, gated_names, fda, min_file: cfg.min_file, ignore, firmlinks }
    }

    /// Whether a folder must not be listed for lack of Full Disk Access.
    pub fn is_gated(&self, path: &Path, name: &str) -> bool {
        if self.gated.is_empty() {
            return false;
        }
        if name.ends_with(".photoslibrary") {
            return true;
        }
        if !self.gated_names.contains(name) {
            return false;
        }
        let s = path.to_string_lossy();
        self.gated.iter().any(|g| {
            s.strip_suffix(g).and_then(|h| h.strip_suffix('/')).map(is_home).unwrap_or(false)
        })
    }

    /// Whether a subfolder is outside the scan entirely (no node at all).
    pub fn skip_dir_pub(&self, dir: &Path, path: &Path, name: &str, dev: u64, mount: bool) -> bool {
        if (name == "Mobile Documents" || name == "CloudStorage") && dir.file_name().map(|n| n == "Library").unwrap_or(false) {
            return true;
        }
        if !self.excluded.is_empty() && self.excluded.contains(path) {
            return true;
        }
        let dev = if mount { std::fs::symlink_metadata(path).map(|m| m.dev() as u64).unwrap_or(u64::MAX) } else { dev };
        !self.devices.contains(&dev)
    }

    /// What FSEvents watches. For "/" that is "/" itself: its stream reports data-volume changes
    /// under their firmlinked paths (/Users, /private …) and replays their history; a stream on
    /// /System/Volumes/Data gets no live events for them.
    pub fn watch_root(&self) -> String {
        self.root_str.clone()
    }

    /// An FSEvents "rescan everything below" for this path means the whole tree.
    pub fn is_whole(&self, p: &str) -> bool {
        p == self.root_str || (self.root_str == "/" && p == "/System/Volumes/Data")
    }

    /// Map an FSEvents path to the tree's path space (None when ignored).
    pub fn event_path(&self, raw: &str) -> Option<String> {
        let p = if raw.len() > 1 { raw.trim_end_matches('/') } else { raw };
        let mut p = p.to_string();
        for (twin, visible) in &self.firmlinks {
            if let Some(rest) = p.strip_prefix(twin.as_str()) {
                if rest.is_empty() || rest.starts_with('/') {
                    p = format!("{}{}", visible, rest);
                    break;
                }
            }
        }
        let with_slash = format!("{}/", p);
        if self.ignore.iter().any(|i| with_slash.starts_with(i.as_str())) {
            return None;
        }
        if p.contains("/Library/Mobile Documents") || p.contains("/Library/CloudStorage") {
            return None;
        }
        strip_root(&self.root_str, &p)?;
        Some(p)
    }
}

/// "/Users/<name>" (or the current $HOME).
fn is_home(h: &str) -> bool {
    if let Some(rest) = h.strip_prefix("/Users/") {
        return !rest.is_empty() && !rest.contains('/');
    }
    let home = disko_core::paths::home();
    Path::new(h) == home
}

#[derive(Default)]
pub struct Stats {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub last_dir: AtomicU32,
    /// Time walkers spent waiting for the tree lock (diagnostics).
    pub lock_wait_ns: AtomicU64,
    pub list_ns: AtomicU64,
}

pub struct Job {
    pub path: PathBuf,
    pub idx: u32,
    /// Inside a matched folder: the matcher isn't consulted below it.
    pub in_match: bool,
}

pub struct Walk<'a> {
    pub tree: &'a RwLock<Tree>,
    pub scope: &'a Scope,
    pub matcher: &'a dyn WalkMatcher,
    pub sink: &'a (dyn Fn(EngineEvent) + Sync),
    pub cancel: &'a AtomicBool,
    pub stats: &'a Stats,
}

thread_local! {
    static LISTING: std::cell::RefCell<Listing> = std::cell::RefCell::new(Listing::default());
}

struct KeptFile {
    ent: usize,
    bytes: u64,
    tag: Option<u32>,
}

/// Key space of `Tree::links` for clone families (hard links use plain (dev, inode)).
const CLONE_KEY: u64 = 1 << 63;

/// Bytes a file that shares storage counts here: hard links count once (in the folder that saw
/// them first, `None` elsewhere); a clone family's data counts once in full and its other members
/// (pure clones, all blocks shared) count 0. Plain files count their allocation.
pub fn counted_bytes(t: &mut Tree, e: &bulk::Ent, dir: u32, here: &mut HashSet<(u64, u64)>) -> Option<u64> {
    if e.nlink > 1 {
        let key = (e.dev, e.ino);
        let owner = *t.links.entry(key).or_insert(dir);
        if (owner != dir && !t.is_dead(owner)) || !here.insert(key) {
            return None;
        }
        t.links.insert(key, dir);
    }
    if e.is_clone() {
        let key = (e.dev | CLONE_KEY, e.clone_id);
        let owner = *t.links.entry(key).or_insert(dir);
        if (owner != dir && !t.is_dead(owner)) || !here.insert(key) {
            // A pure clone of data already counted: it holds no blocks of its own.
            return Some(0);
        }
        t.links.insert(key, dir);
    }
    Some(e.alloc)
}

/// Whether the file needs `counted_bytes` (shared storage) or simply counts its allocation.
#[inline]
pub fn shares(e: &bulk::Ent) -> bool {
    e.nlink > 1 || e.is_clone()
}

struct SubDir {
    ent: usize,
    path: PathBuf,
    gated: bool,
    tag: Option<u32>,
}

impl<'a> Walk<'a> {
    /// Walk `jobs` (folders already in the tree with `pending == 1`) to completion.
    pub fn run(&self, pool: &rayon::ThreadPool, jobs: Vec<Job>) {
        pool.scope(|s| {
            for j in jobs {
                s.spawn(move |s| self.dir(s, j));
            }
        });
    }

    fn dir<'s>(&'s self, s: &rayon::Scope<'s>, job: Job) where 'a: 's {
        if self.cancel.load(Ordering::Relaxed) {
            return;
        }
        let jobs = LISTING.with(|cell| {
            let mut listing = cell.borrow_mut();
            self.process(&job, &mut listing)
        });
        for j in jobs {
            s.spawn(move |s| self.dir(s, j));
        }
    }

    fn process(&self, job: &Job, l: &mut Listing) -> Vec<Job> {
        let t0 = std::time::Instant::now();
        let res = bulk::list(&job.path, l);
        self.stats.list_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.stats.dirs.fetch_add(1, Ordering::Relaxed);
        self.stats.last_dir.store(job.idx, Ordering::Relaxed);
        let mut matched = Vec::new();
        let jobs = match res {
            Err(e) => {
                let code = err_code(e.raw_os_error().unwrap_or(libc::EIO), self.scope.fda);
                let mut t = self.tree.write().unwrap_or_else(std::sync::PoisonError::into_inner);
                t.err[job.idx as usize] = code;
                t.finish(job.idx, &mut matched);
                Vec::new()
            }
            Ok(()) => self.insert(job, l, &mut matched),
        };
        for m in matched {
            (self.sink)(EngineEvent::Matched(m));
        }
        jobs
    }

    /// Fold a listing into the tree under `job.idx` and return the subfolders to walk.
    fn insert(&self, job: &Job, l: &Listing, matched: &mut Vec<Matched>) -> Vec<Job> {
        let min = self.scope.min_file;
        let (mut own_b, mut own_f, mut own_n) = (0u64, 0u32, 0u32);
        let mut kept: Vec<KeptFile> = Vec::new();
        let mut links: Vec<usize> = Vec::new();
        let mut subdirs: Vec<SubDir> = Vec::new();
        let mut siblings: Option<Vec<&str>> = None;
        for (i, e) in l.ents.iter().enumerate() {
            if e.err != 0 {
                continue;
            }
            match e.kind {
                Kind::Dir => {
                    let name = l.name(e);
                    let path = job.path.join(name);
                    if self.scope.skip_dir_pub(&job.path, &path, name, e.dev, e.mount) {
                        continue;
                    }
                    let gated = self.scope.is_gated(&path, name);
                    let tag = if job.in_match || gated {
                        None
                    } else {
                        let sib = siblings.get_or_insert_with(|| l.ents.iter().map(|x| l.name(x)).collect());
                        self.matcher.on_dir(&path, name, sib)
                    };
                    subdirs.push(SubDir { ent: i, path, gated, tag });
                }
                _ => {
                    if shares(e) {
                        links.push(i);
                    } else if e.alloc >= min && e.kind == Kind::File {
                        kept.push(KeptFile { ent: i, bytes: e.alloc, tag: None });
                    } else {
                        own_b += e.alloc;
                        own_f += 1;
                        own_n = own_n.max(secs(e.mtime));
                    }
                }
            }
        }
        // Matcher for big files, outside the lock.
        if !job.in_match {
            for k in kept.iter_mut() {
                let e = &l.ents[k.ent];
                let name = l.name(e);
                k.tag = self.matcher.on_file(&job.path.join(name), name, k.bytes, e.mtime);
            }
        }
        let t0 = std::time::Instant::now();
        let mut t = self.tree.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.stats.lock_wait_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        // Hard links: count each (dev, inode) once, in the folder that saw it first.
        let mut seen_here: HashSet<(u64, u64)> = HashSet::new();
        for &i in &links {
            let e = &l.ents[i];
            let Some(bytes) = counted_bytes(&mut t, e, job.idx, &mut seen_here) else { continue };
            if bytes >= min && e.kind == Kind::File {
                let name = l.name(e);
                let tag = if job.in_match { None } else { self.matcher.on_file(&job.path.join(name), name, bytes, e.mtime) };
                kept.push(KeptFile { ent: i, bytes, tag });
            } else {
                own_b += bytes;
                own_f += 1;
                own_n = own_n.max(secs(e.mtime));
            }
        }
        let (mut add_b, mut add_f, mut add_n) = (own_b, own_f as i64, own_n);
        let d = job.idx as usize;
        t.own_bytes[d] += own_b;
        t.own_files[d] += own_f;
        t.own_newest[d] = t.own_newest[d].max(own_n);
        for k in &kept {
            let e = &l.ents[k.ent];
            let c = t.add_child(job.idx, l.name(e), F_FILE);
            let cu = c as usize;
            let n = secs(e.mtime);
            t.own_bytes[cu] = k.bytes;
            t.own_files[cu] = 1;
            t.own_newest[cu] = n;
            t.bytes[cu] = k.bytes;
            t.files[cu] = 1;
            t.newest[cu] = n;
            add_b += k.bytes;
            add_f += 1;
            add_n = add_n.max(n);
            if let Some(tag) = k.tag {
                t.flags[cu] |= F_MATCHED;
                t.tags.insert(c, tag);
                t.cat[cu] = cat_code(self.matcher.category(tag));
                matched.push(t.matched(c));
            }
        }
        t.add_up(job.idx, add_b as i64, add_f, add_n);
        let mut jobs = Vec::with_capacity(subdirs.len());
        for sd in subdirs {
            let c = t.add_child(job.idx, l.name(&l.ents[sd.ent]), F_DIR);
            if sd.gated {
                t.err[c as usize] = E_FDA;
                continue;
            }
            if let Some(tag) = sd.tag {
                t.flags[c as usize] |= F_MATCHED;
                t.tags.insert(c, tag);
                t.cat[c as usize] = cat_code(self.matcher.category(tag));
            }
            t.set_pending(c, 1);
            jobs.push(Job { path: sd.path, idx: c, in_match: job.in_match || sd.tag.is_some() });
        }
        t.add_pending(job.idx, jobs.len() as u32);
        t.finish(job.idx, matched);
        drop(t);
        self.stats.files.fetch_add(add_f as u64, Ordering::Relaxed);
        self.stats.bytes.fetch_add(add_b, Ordering::Relaxed);
        jobs
    }
}

/// Best-effort Full Disk Access probe (opens and closes a TCC database; reads nothing).
pub fn has_full_disk_access() -> bool {
    let home = disko_core::paths::home();
    std::fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
        || std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gating_only_under_homes() {
        let cfg = crate::EngineConfig { root: PathBuf::from("/tmp"), ..Default::default() };
        let s = Scope::new(&cfg, false, Path::new("/nonexistent/snap"));
        assert!(s.is_gated(Path::new("/Users/x/Library/Mail"), "Mail"));
        assert!(!s.is_gated(Path::new("/Users/x/Documents"), "Documents"));
        assert!(s.is_gated(Path::new("/Users/x/Library/Containers"), "Containers"));
        assert!(!s.is_gated(Path::new("/Users/x/Projects/Mail"), "Mail"));
        assert!(!s.is_gated(Path::new("/Users/x/a/Containers"), "Containers"));
        assert!(s.is_gated(Path::new("/Users/x/Pictures/Photos Library.photoslibrary"), "Photos Library.photoslibrary"));
        let s = Scope::new(&crate::EngineConfig { root: PathBuf::from("/tmp"), allow_prompts: true, ..Default::default() }, false, Path::new("/x"));
        assert!(!s.is_gated(Path::new("/Users/x/Library/Containers"), "Containers"));
        assert!(!s.is_gated(Path::new("/Users/x/Documents"), "Documents"));
        let s = Scope::new(&crate::EngineConfig { root: PathBuf::from("/tmp"), user_folders: false, ..Default::default() }, false, Path::new("/x"));
        assert!(s.is_gated(Path::new("/Users/x/Documents"), "Documents"));
        let s = Scope::new(&cfg, true, Path::new("/x"));
        assert!(!s.is_gated(Path::new("/Users/x/Library/Mail"), "Mail"));
    }

    #[test]
    fn event_paths_map_firmlinks() {
        let mut s = Scope::new(&crate::EngineConfig::default(), true, Path::new("/Users/x/Library/Caches/app.disko/snapshot.bin"));
        s.firmlinks = vec![("/System/Volumes/Data/Users".into(), "/Users".into())];
        assert_eq!(s.event_path("/System/Volumes/Data/Users/x/a/").as_deref(), Some("/Users/x/a"));
        assert_eq!(s.event_path("/System/Volumes/Data/.Spotlight-V100").as_deref(), Some("/System/Volumes/Data/.Spotlight-V100"));
        assert_eq!(s.event_path("/Volumes/USB/x"), None);
        assert_eq!(s.event_path("/Users/x/Library/Caches/app.disko"), None);
        assert_eq!(s.event_path("/Users/x/Library/Mobile Documents/foo"), None);
    }
}
