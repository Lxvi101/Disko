//! Parallel filesystem scanner that writes the same SQLite layout as `disko_cli.core.scan`,
//! so the Python analysis commands (candidates, unused, plan…) keep working on its output.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub output: PathBuf,
    pub quick: bool,
    pub excludes: Vec<PathBuf>,
    pub home: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Progress {
    pub entries: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub current: String,
    pub phase: String,
    pub elapsed_ms: u64,
    /// While walking: the biggest folders so far, two levels deep, so the UI can draw the map as it forms.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<LiveFolder>,
    /// While walking: rebuildable folders spotted so far, by kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub junk: Vec<LiveJunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveFolder {
    pub path: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LiveFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveJunk {
    pub kind: String,
    pub bytes: u64,
    pub count: u32,
}

/// Rebuildable folder kinds recognised by name while walking (index 0 is "none").
const JUNK_KINDS: [&str; 6] = ["", "node_modules", "build", "python", "caches", "trash"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub output: PathBuf,
    pub entries: u64,
    pub rows: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub coverage_issues: u64,
}

const QUICK_MIN_FILE: u64 = 256 * 1024;

struct RawChild {
    name: String,
    token: u64, // non-zero for directories that will be traversed
    kind: u8,   // 0 dir, 1 file, 2 symlink, 3 special, 4 unknown
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
    /// Index of the root's child this entry sits under, and of the grandchild (u32::MAX when none).
    top: u32,
    sub: u32,
    /// JUNK_KINDS index of the rebuildable folder this entry sits in (0 when none).
    junk: u8,
}

fn kind_str(k: u8) -> &'static str {
    match k {
        0 => "directory",
        1 => "file",
        2 => "symlink",
        3 => "special",
        _ => "unknown",
    }
}

fn now_f64() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn stat_child(path: &Path, name: String, token: u64) -> RawChild {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let ft = m.file_type();
            let kind = if ft.is_dir() { 0 } else if ft.is_file() { 1 } else if ft.is_symlink() { 2 } else { 3 };
            RawChild {
                name,
                token: if kind == 0 { token } else { 0 },
                kind,
                dev: m.dev(),
                ino: m.ino(),
                mode: m.mode(),
                size: m.size(),
                mtime_ns: m.mtime() * 1_000_000_000 + m.mtime_nsec(),
                ctime_ns: m.ctime() * 1_000_000_000 + m.ctime_nsec(),
                allocated: m.blocks() * 512,
                nlink: m.nlink(),
                error: None,
            }
        }
        Err(e) => RawChild { name, token: 0, kind: 4, dev: 0, ino: 0, mode: 0, size: 0, mtime_ns: 0, ctime_ns: 0, allocated: 0, nlink: 0, error: Some(e.to_string()) },
    }
}

/// Folders guarded by macOS privacy protections. Listing them without Full Disk Access raises a
/// prompt each, so they are skipped (and reported) when the process lacks that permission.
const TCC_GATED: [&str; 26] = [
    "Library/Application Support/AddressBook",
    "Library/Application Support/CallHistoryDB",
    "Library/Application Support/CallHistoryTransactions",
    "Library/Application Support/com.apple.TCC",
    "Library/Application Support/com.apple.sharedfilelist",
    "Library/Application Support/Knowledge",
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
];

fn is_tcc_gated(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.ends_with(".photoslibrary") {
        return true;
    }
    TCC_GATED.iter().any(|g| s.ends_with(&format!("/{}", g)))
}

struct Walker {
    skip_tcc: bool,
    tx: mpsc::Sender<(u64, PathBuf, Vec<RawChild>, Option<String>)>,
    next_token: AtomicU64,
    devices: Vec<u64>,
    excludes: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
    dirs: Arc<AtomicU64>,
}

impl Walker {
    fn walk_dir<'s>(&'s self, scope: &rayon::Scope<'s>, dir: PathBuf, token: u64) {
        if self.cancel.load(Ordering::Relaxed) {
            return;
        }
        self.dirs.fetch_add(1, Ordering::Relaxed);
        let listed = match crate::bulkdir::list(&dir) {
            Ok(list) => Ok(list.into_iter().map(|e| RawChild { name: e.name, token: 0, kind: e.kind, dev: e.dev, ino: e.ino, mode: e.mode, size: e.size, mtime_ns: e.mtime_ns, ctime_ns: e.ctime_ns, allocated: e.allocated, nlink: e.nlink, error: e.error }).collect::<Vec<_>>()),
            // Volumes without bulk listing (some network and FUSE file systems): readdir + lstat.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EINVAL) | Some(libc::ENOSYS)) => match fs::read_dir(&dir) {
                Ok(rd) => Ok(rd.flatten().map(|ent| stat_child(&ent.path(), ent.file_name().to_string_lossy().to_string(), 0)).collect()),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        let mut children = match listed {
            Ok(c) => c,
            Err(e) => {
                let _ = self.tx.send((token, dir, Vec::new(), Some(e.to_string())));
                return;
            }
        };
        // Only folders that contain an exclude need the per-entry check.
        let excludes: Vec<&PathBuf> = self.excludes.iter().filter(|x| x.starts_with(&dir) && x.as_path() != dir).collect();
        let all_excluded = self.excludes.iter().any(|x| dir.starts_with(x));
        let cloud_parent = dir.file_name().map(|n| n == "Library").unwrap_or(false);
        let mut subdirs = Vec::new();
        for c in children.iter_mut() {
            let excluded = all_excluded
                || (cloud_parent && c.kind == 0 && (c.name == "Mobile Documents" || c.name == "CloudStorage"))
                || (!excludes.is_empty() && excludes.iter().any(|x| x.as_path() == dir.join(&c.name)));
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

/// Run a scan. `progress` is called periodically from the collector thread.
pub fn scan(opts: ScanOptions, cancel: Arc<AtomicBool>, progress: &mut dyn FnMut(Progress)) -> Result<ScanSummary, String> {
    let start = Instant::now();
    let started = now_f64();
    let root = fs::canonicalize(&opts.root).map_err(|e| format!("Scan root: {}", e))?;
    let root_meta = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
    if !root_meta.is_dir() {
        return Err("Scan root must be a directory".into());
    }
    if opts.output.exists() {
        return Err("Output already exists; choose a new scan database.".into());
    }

    // Devices we may descend into: the root's device plus the APFS data volume when scanning '/'.
    let mut devices = vec![root_meta.dev()];
    let whole_disk = root == Path::new("/");
    if whole_disk {
        for extra in ["/System/Volumes/Data", "/System/Volumes/VM"] {
            if let Ok(m) = fs::symlink_metadata(extra) {
                devices.push(m.dev());
            }
        }
    }

    let mut excludes: Vec<PathBuf> = vec![
        opts.home.join("Library/Mobile Documents"),
        opts.home.join("Library/CloudStorage"),
        PathBuf::from("/Volumes"),
        PathBuf::from("/System/Volumes/Preboot"),
        PathBuf::from("/System/Volumes/Update"),
        PathBuf::from("/System/Volumes/Hardware"),
        PathBuf::from("/System/Volumes/xarts"),
        PathBuf::from("/System/Volumes/iSCPreboot"),
        PathBuf::from("/dev"),
    ];
    // Firmlinks expose the data volume under /Users, /Applications, … so skip their twins inside
    // /System/Volumes/Data to avoid counting everything twice. Data-only folders there still count.
    if whole_disk {
        if let Ok(list) = fs::read_to_string("/usr/share/firmlinks") {
            for line in list.lines() {
                if let Some((_, target)) = line.split_once('\t') {
                    excludes.push(PathBuf::from("/System/Volumes/Data").join(target.trim()));
                }
            }
        }
    }
    excludes.extend(opts.excludes.iter().cloned());
    for suffix in ["", "-journal", "-wal", "-shm"] {
        excludes.push(PathBuf::from(format!("{}{}", opts.output.display(), suffix)));
    }

    // ---- traverse ----
    let (tx, rx) = mpsc::channel();
    let dirs = Arc::new(AtomicU64::new(0));
    let walker = Walker {
        skip_tcc: !crate::disk_access::has_full_disk_access(&opts.home),
        tx,
        next_token: AtomicU64::new(1),
        devices,
        excludes: excludes.clone(),
        cancel: cancel.clone(),
        dirs: dirs.clone(),
    };

    let root_entry = Entry {
        name: root.to_string_lossy().into(),
        parent: u32::MAX,
        kind: 0,
        dev: root_meta.dev(),
        ino: root_meta.ino(),
        mode: root_meta.mode(),
        size: root_meta.size(),
        mtime_ns: root_meta.mtime() * 1_000_000_000 + root_meta.mtime_nsec(),
        ctime_ns: root_meta.ctime() * 1_000_000_000 + root_meta.ctime_nsec(),
        allocated: root_meta.blocks() * 512,
        logical: 0,
        total: root_meta.blocks() * 512,
        error: None,
        small_count: 0,
        top: u32::MAX,
        sub: u32::MAX,
        junk: 0,
    };
    let mut entries: Vec<Entry> = vec![root_entry];
    let mut token_index: HashMap<u64, u32> = HashMap::new();
    token_index.insert(0, 0);
    let mut inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut seen: u64 = 1;
    let mut bytes: u64 = 0;
    let mut last_report = Instant::now();
    let quick = opts.quick;
    // Live totals for the preview: per top-level entry, per second-level entry, per junk kind.
    let mut top_bytes: HashMap<u32, u64> = HashMap::new();
    let mut sub_bytes: HashMap<u32, u64> = HashMap::new();
    let mut junk_bytes = [0u64; JUNK_KINDS.len()];
    let mut junk_count = [0u32; JUNK_KINDS.len()];
    let home_lib = opts.home.join("Library");

    let root2 = root.clone();
    let traversal = std::thread::spawn(move || {
        let walker = walker; // moved in; dropping it closes the channel when traversal ends
        rayon::scope(|s| {
            walker.walk_dir(s, root2, 0);
        });
        drop(walker);
    });

    // Collector: assign indices as batches arrive (parents always precede children).
    for (parent_token, dir_path, children, dir_error) in rx.iter() {
        let Some(&pidx) = token_index.get(&parent_token) else { continue };
        if let Some(e) = dir_error {
            entries[pidx as usize].error = Some(e.into());
        }
        let (ptop, psub, pjunk, pdepth) = {
            let p = &entries[pidx as usize];
            (p.top, p.sub, p.junk, if pidx == 0 { 0 } else if p.parent == 0 { 1 } else { 2 })
        };
        // Sibling markers decide whether `target` / `.build` are build output.
        let sibling = |n: &str| children.iter().any(|c| c.name == n);
        let (cargo, swiftpm) = if pjunk == 0 && children.iter().any(|c| c.kind == 0 && (c.name == "target" || c.name == ".build")) {
            (sibling("Cargo.toml"), sibling("Package.swift"))
        } else {
            (false, false)
        };
        let in_home = dir_path == opts.home;
        let in_lib = dir_path == home_lib;
        for c in children {
            seen += 1;
            let mut allocated = if c.error.is_some() { 0 } else { c.allocated };
            if c.kind == 1 && c.nlink > 1 && allocated > 0 && !inodes.insert((c.dev, c.ino)) {
                allocated = 0;
            }
            let logical = if c.kind == 1 && c.error.is_none() { c.size } else { 0 };
            bytes += allocated;
            let idx = entries.len() as u32;
            let (top, sub) = match pdepth { 0 => (idx, u32::MAX), 1 => (ptop, idx), _ => (ptop, psub) };
            let junk = if pjunk != 0 || c.kind != 0 {
                pjunk
            } else {
                let k = match c.name.as_str() {
                    "node_modules" => 1,
                    "DerivedData" | ".next" | ".nuxt" | ".turbo" | ".gradle" | ".parcel-cache" | ".svelte-kit" => 2,
                    "target" if cargo => 2,
                    ".build" if swiftpm => 2,
                    ".venv" | "venv" | "__pycache__" | ".mypy_cache" | ".pytest_cache" | ".tox" => 3,
                    "Caches" if in_lib => 4,
                    ".cache" | ".npm" | ".bun" if in_home => 4,
                    ".Trash" if in_home => 5,
                    _ => 0,
                };
                if k != 0 {
                    junk_count[k as usize] += 1;
                }
                k
            };
            if allocated > 0 {
                // Small files that a quick scan folds into their folder have no index of their own.
                let folded = quick && c.kind == 1 && c.error.is_none() && c.size < QUICK_MIN_FILE;
                let t = if folded { ptop } else { top };
                let s = if folded { psub } else { sub };
                if t != u32::MAX { *top_bytes.entry(t).or_default() += allocated; }
                if s != u32::MAX { *sub_bytes.entry(s).or_default() += allocated; }
                junk_bytes[junk as usize] += allocated;
            }
            if quick && c.kind == 1 && c.error.is_none() && c.size < QUICK_MIN_FILE {
                let p = &mut entries[pidx as usize];
                p.allocated += allocated;
                p.total += allocated;
                p.logical += logical;
                p.small_count += 1;
                continue;
            }
            if c.token != 0 {
                token_index.insert(c.token, idx);
            }
            entries.push(Entry {
                name: c.name.into(),
                parent: pidx,
                kind: c.kind,
                dev: c.dev,
                ino: c.ino,
                mode: c.mode,
                size: c.size,
                mtime_ns: c.mtime_ns,
                ctime_ns: c.ctime_ns,
                allocated,
                logical,
                total: allocated,
                error: c.error.map(|e| e.into()),
                small_count: 0,
                top,
                sub,
                junk,
            });
        }
        if last_report.elapsed().as_millis() > 250 {
            last_report = Instant::now();
            progress(Progress {
                entries: seen,
                dirs: dirs.load(Ordering::Relaxed),
                bytes,
                current: dir_path.to_string_lossy().to_string(),
                phase: "scan".into(),
                elapsed_ms: start.elapsed().as_millis() as u64,
                top: live_tree(&root, &entries, &top_bytes, &sub_bytes),
                junk: live_junk(&junk_bytes, &junk_count),
            });
        }
    }
    let _ = traversal.join();
    if cancel.load(Ordering::Relaxed) {
        return Err("Scan cancelled".into());
    }

    // ---- aggregate ----
    let snapshot = |phase: &str, current: String, entries: &[Entry]| Progress {
        entries: seen,
        dirs: dirs.load(Ordering::Relaxed),
        bytes,
        current,
        phase: phase.into(),
        elapsed_ms: start.elapsed().as_millis() as u64,
        top: live_tree(&root, entries, &top_bytes, &sub_bytes),
        junk: live_junk(&junk_bytes, &junk_count),
    };
    progress(snapshot("aggregate", String::new(), &entries));
    for i in (1..entries.len()).rev() {
        let (t, l, p) = {
            let e = &entries[i];
            (e.total, e.logical, e.parent as usize)
        };
        let parent = &mut entries[p];
        parent.total += t;
        parent.logical += l;
    }

    // ---- write ----
    progress(snapshot("write", String::new(), &entries));
    let conn = Connection::open(&opts.output).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&opts.output, fs::Permissions::from_mode(0o600));
    }
    // `path` gets its unique index after the bulk insert: building it once is much faster than
    // keeping a b-tree of millions of strings balanced row by row.
    conn.execute_batch(
        "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -200000; PRAGMA page_size = 8192; PRAGMA threads = 4;
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE entries(id INTEGER PRIMARY KEY, parent INTEGER, path TEXT,
           kind TEXT, device INTEGER, inode INTEGER, mode INTEGER, size INTEGER,
           mtime_ns INTEGER, ctime_ns INTEGER, allocated INTEGER, total INTEGER,
           logical INTEGER, category TEXT, reason TEXT, error TEXT, tag TEXT, action TEXT, cost TEXT);
         CREATE TABLE inodes(device INTEGER, inode INTEGER, PRIMARY KEY(device,inode));",
    )
    .map_err(|e| e.to_string())?;

    let volume = crate::volume_info(&root);
    let home_str = opts.home.to_string_lossy().to_string();
    let meta_put = |conn: &Connection, k: &str, v: serde_json::Value| -> Result<(), String> {
        conn.execute("INSERT OR REPLACE INTO meta VALUES (?1, ?2)", params![k, v.to_string()]).map(|_| ()).map_err(|e| e.to_string())
    };
    meta_put(&conn, "schema", serde_json::json!(1))?;
    meta_put(&conn, "root", serde_json::json!(root.to_string_lossy()))?;
    meta_put(&conn, "started", serde_json::json!(started))?;
    meta_put(&conn, "complete", serde_json::json!(false))?;
    meta_put(&conn, "home", serde_json::json!(home_str))?;
    meta_put(&conn, "excludes", serde_json::json!(excludes.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()))?;
    meta_put(&conn, "volume", serde_json::json!({ "total": volume.total, "available": volume.available }))?;
    meta_put(&conn, "scanner", serde_json::json!("disko"))?;
    meta_put(&conn, "mode", serde_json::json!(if quick { "quick" } else { "thorough" }))?;

    // Paths are built here in order (parents come first), classification runs in parallel per
    // chunk, and a writer thread inserts the previous chunk meanwhile.
    struct Row {
        path: String,
        cls: crate::classify::Classification,
    }
    const CHUNK: usize = 50_000;
    let total_rows = entries.len();
    let (wtx, wrx) = mpsc::sync_channel::<(usize, Vec<Row>)>(3);
    let entries = Arc::new(entries);
    let writer = {
        let entries = entries.clone();
        let cancel = cancel.clone();
        std::thread::spawn(move || -> Result<(Connection, u64), String> {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let mut coverage = 0u64;
            {
                let mut stmt = tx
                    .prepare("INSERT INTO entries(id,parent,path,kind,device,inode,mode,size,mtime_ns,ctime_ns,allocated,total,logical,category,reason,error,tag,action,cost) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)")
                    .map_err(|e| e.to_string())?;
                for (first, rows) in wrx.iter() {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Scan cancelled".into());
                    }
                    for (k, row) in rows.into_iter().enumerate() {
                        let i = first + k;
                        let e = &entries[i];
                        if e.error.is_some() {
                            coverage += 1;
                        }
                        let parent: Option<i64> = if e.parent == u32::MAX { None } else { Some(e.parent as i64 + 1) };
                        stmt.execute(params![
                            i as i64 + 1,
                            parent,
                            row.path,
                            kind_str(e.kind),
                            e.dev as i64,
                            e.ino as i64,
                            e.mode as i64,
                            e.size as i64,
                            e.mtime_ns,
                            e.ctime_ns,
                            e.allocated as i64,
                            e.total as i64,
                            e.logical as i64,
                            row.cls.category,
                            row.cls.reason,
                            e.error.as_deref(),
                            row.cls.tag,
                            row.cls.action,
                            row.cls.cost,
                        ])
                        .map_err(|err| format!("{} ({})", err, row.path))?;
                    }
                }
            }
            drop(entries); // the index sorts below need the memory more
            tx.execute_batch(
                "CREATE UNIQUE INDEX paths ON entries(path);
                 CREATE INDEX parents ON entries(parent, total DESC);
                 CREATE INDEX totals ON entries(total);
                 CREATE INDEX cats ON entries(category, total DESC);
                 CREATE INDEX tags ON entries(tag, total DESC);",
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok((conn, coverage))
        })
    };

    let mut dir_paths: HashMap<u32, Arc<str>> = HashMap::new();
    let mut produce = || -> Result<(), String> {
        use rayon::prelude::*;
        let mut first = 0;
        while first < total_rows {
            if cancel.load(Ordering::Relaxed) {
                return Err("Scan cancelled".into());
            }
            let last = (first + CHUNK).min(total_rows);
            let mut paths = Vec::with_capacity(last - first);
            for i in first..last {
                let e = &entries[i];
                let path = if e.parent == u32::MAX {
                    e.name.to_string()
                } else {
                    let pp = dir_paths.get(&e.parent).map(|s| &**s).unwrap_or("");
                    if pp == "/" { format!("/{}", e.name) } else { format!("{}/{}", pp, e.name) }
                };
                if e.kind == 0 {
                    dir_paths.insert(i as u32, Arc::from(path.as_str()));
                }
                paths.push(path);
            }
            let rows: Vec<Row> = paths.into_par_iter().with_min_len(512).map(|path| {
                let cls = crate::classify::classify(&path, &home_str);
                Row { path, cls }
            }).collect();
            if wtx.send((first, rows)).is_err() {
                break; // the writer failed; its error is reported below
            }
            if first > 0 && first % 200_000 == 0 {
                progress(Progress { entries: seen, dirs: dirs.load(Ordering::Relaxed), bytes, current: format!("{} of {} rows", first, total_rows), phase: "write".into(), elapsed_ms: start.elapsed().as_millis() as u64, ..Default::default() });
            }
            first = last;
        }
        Ok(())
    };
    let produced = produce();
    drop(produce);
    drop(dir_paths);
    drop(inodes);
    drop(token_index);
    drop(wtx);
    drop(entries);
    let written = writer.join().map_err(|_| "Database writer crashed".to_string())?;
    let (conn, coverage) = match (produced, written) {
        (Ok(()), Ok(v)) => v,
        (Err(e), _) | (_, Err(e)) => {
            let _ = fs::remove_file(&opts.output);
            return Err(e);
        }
    };
    meta_put(&conn, "complete", serde_json::json!(true))?;
    meta_put(&conn, "finished", serde_json::json!(now_f64()))?;
    meta_put(&conn, "entries", serde_json::json!(seen))?;
    meta_put(&conn, "rows", serde_json::json!(total_rows))?;
    meta_put(&conn, "coverage_issues", serde_json::json!(coverage))?;
    meta_put(&conn, "rules", serde_json::json!(crate::classify::RULES_VERSION))?;
    Ok(ScanSummary {
        output: opts.output.clone(),
        entries: seen,
        rows: total_rows as u64,
        bytes,
        elapsed_ms: start.elapsed().as_millis() as u64,
        coverage_issues: coverage,
    })
}

/// The eight biggest top-level entries with their four biggest children, from the live totals.
fn live_tree(root: &Path, entries: &[Entry], tops: &HashMap<u32, u64>, subs: &HashMap<u32, u64>) -> Vec<LiveFolder> {
    let join = |base: &str, name: &str| if base.ends_with('/') { format!("{}{}", base, name) } else { format!("{}/{}", base, name) };
    let root_s = root.to_string_lossy();
    let mut t: Vec<(u32, u64)> = tops.iter().map(|(&k, &v)| (k, v)).collect();
    t.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    t.truncate(8);
    let mut kids: HashMap<u32, Vec<(u32, u64)>> = HashMap::new();
    for (&k, &v) in subs {
        let parent = entries[k as usize].top;
        if t.iter().any(|(p, _)| *p == parent) {
            kids.entry(parent).or_default().push((k, v));
        }
    }
    t.into_iter()
        .map(|(k, v)| {
            let path = join(&root_s, &entries[k as usize].name);
            let mut c = kids.remove(&k).unwrap_or_default();
            c.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            c.truncate(4);
            let children = c.into_iter().map(|(ck, cv)| LiveFolder { path: join(&path, &entries[ck as usize].name), bytes: cv, children: vec![] }).collect();
            LiveFolder { path, bytes: v, children }
        })
        .collect()
}

fn live_junk(bytes: &[u64], count: &[u32]) -> Vec<LiveJunk> {
    (1..JUNK_KINDS.len()).filter(|&k| bytes[k] > 0).map(|k| LiveJunk { kind: JUNK_KINDS[k].into(), bytes: bytes[k], count: count[k] }).collect()
}
