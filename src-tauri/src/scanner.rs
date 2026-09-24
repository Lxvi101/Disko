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
}

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
    blocks: u64,
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

/// iCloud Drive and File Provider roots: `<anything>/Library/Mobile Documents`, `<anything>/Library/CloudStorage`.
/// Opening them triggers provider downloads and permission prompts, so they are excluded by name.
fn is_cloud_dir(parent: &Path, name: &str) -> bool {
    (name == "Mobile Documents" || name == "CloudStorage") && parent.file_name().map(|n| n == "Library").unwrap_or(false)
}

fn is_inside(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
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
                blocks: m.blocks(),
                nlink: m.nlink(),
                error: None,
            }
        }
        Err(e) => RawChild { name, token: 0, kind: 4, dev: 0, ino: 0, mode: 0, size: 0, mtime_ns: 0, ctime_ns: 0, blocks: 0, nlink: 0, error: Some(e.to_string()) },
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
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                let _ = self.tx.send((token, dir, Vec::new(), Some(e.to_string())));
                return;
            }
        };
        let mut children = Vec::new();
        let mut subdirs = Vec::new();
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            let path = ent.path();
            let child_token = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
            let mut c = stat_child(&path, name, child_token);
            if c.kind == 0 {
                let cloud = is_cloud_dir(&dir, &c.name);
                let excluded = cloud || self.excludes.iter().any(|x| is_inside(&path, x));
                let cross = !self.devices.contains(&c.dev);
                if excluded {
                    c.error = Some("Excluded from scan.".into());
                    c.token = 0;
                } else if self.skip_tcc && is_tcc_gated(&path) {
                    c.error = Some("Needs Full Disk Access.".into());
                    c.token = 0;
                } else if cross {
                    c.error = Some("Different filesystem; not traversed.".into());
                    c.token = 0;
                } else {
                    subdirs.push((path, child_token));
                }
            } else if self.excludes.iter().any(|x| is_inside(&path, x)) {
                c.error = Some("Excluded from scan.".into());
                c.blocks = 0;
            }
            children.push(c);
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
    };
    let mut entries: Vec<Entry> = vec![root_entry];
    let mut token_index: HashMap<u64, u32> = HashMap::new();
    token_index.insert(0, 0);
    let mut inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut seen: u64 = 1;
    let mut bytes: u64 = 0;
    let mut last_report = Instant::now();
    let quick = opts.quick;

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
        for c in children {
            seen += 1;
            let mut allocated = if c.error.is_some() { 0 } else { c.blocks * 512 };
            if c.kind == 1 && c.nlink > 1 && allocated > 0 && !inodes.insert((c.dev, c.ino)) {
                allocated = 0;
            }
            let logical = if c.kind == 1 && c.error.is_none() { c.size } else { 0 };
            bytes += allocated;
            if quick && c.kind == 1 && c.error.is_none() && c.size < QUICK_MIN_FILE {
                let p = &mut entries[pidx as usize];
                p.allocated += allocated;
                p.total += allocated;
                p.logical += logical;
                p.small_count += 1;
                continue;
            }
            let idx = entries.len() as u32;
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
            });
        }
    }
    let _ = traversal.join();
    if cancel.load(Ordering::Relaxed) {
        return Err("Scan cancelled".into());
    }

    // ---- aggregate ----
    progress(Progress { entries: seen, dirs: dirs.load(Ordering::Relaxed), bytes, current: String::new(), phase: "aggregate".into(), elapsed_ms: start.elapsed().as_millis() as u64 });
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
    progress(Progress { entries: seen, dirs: dirs.load(Ordering::Relaxed), bytes, current: String::new(), phase: "write".into(), elapsed_ms: start.elapsed().as_millis() as u64 });
    let conn = Connection::open(&opts.output).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&opts.output, fs::Permissions::from_mode(0o600));
    }
    conn.execute_batch(
        "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -200000;
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE entries(id INTEGER PRIMARY KEY, parent INTEGER, path TEXT UNIQUE,
           kind TEXT, device INTEGER, inode INTEGER, mode INTEGER, size INTEGER,
           mtime_ns INTEGER, ctime_ns INTEGER, allocated INTEGER, total INTEGER,
           logical INTEGER, category TEXT, reason TEXT, error TEXT, tag TEXT, action TEXT, cost TEXT);
         CREATE TABLE inodes(device INTEGER, inode INTEGER, PRIMARY KEY(device,inode));",
    )
    .map_err(|e| e.to_string())?;

    let volume = crate::volume_info(&root);
    let home_str = opts.home.to_string_lossy().to_string();
    let meta_put = |k: &str, v: serde_json::Value| -> Result<(), String> {
        conn.execute("INSERT OR REPLACE INTO meta VALUES (?1, ?2)", params![k, v.to_string()]).map(|_| ()).map_err(|e| e.to_string())
    };
    meta_put("schema", serde_json::json!(1))?;
    meta_put("root", serde_json::json!(root.to_string_lossy()))?;
    meta_put("started", serde_json::json!(started))?;
    meta_put("complete", serde_json::json!(false))?;
    meta_put("home", serde_json::json!(home_str))?;
    meta_put("excludes", serde_json::json!(excludes.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()))?;
    meta_put("volume", serde_json::json!({ "total": volume.total, "available": volume.available }))?;
    meta_put("scanner", serde_json::json!("disko"))?;
    meta_put("mode", serde_json::json!(if quick { "quick" } else { "thorough" }))?;

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare("INSERT INTO entries(id,parent,path,kind,device,inode,mode,size,mtime_ns,ctime_ns,allocated,total,logical,category,reason,error,tag,action,cost) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)")
            .map_err(|e| e.to_string())?;
        let mut dir_paths: HashMap<u32, String> = HashMap::new();
        let mut coverage = 0u64;
        for (i, e) in entries.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                drop(stmt);
                drop(tx);
                let _ = fs::remove_file(&opts.output);
                return Err("Scan cancelled".into());
            }
            let path = if e.parent == u32::MAX {
                e.name.to_string()
            } else {
                let pp = dir_paths.get(&e.parent).map(|s| s.as_str()).unwrap_or("");
                if pp == "/" { format!("/{}", e.name) } else { format!("{}/{}", pp, e.name) }
            };
            if e.kind == 0 {
                dir_paths.insert(i as u32, path.clone());
            }
            let cls = crate::classify::classify(&path, &home_str);
            let error: Option<String> = match (&e.error, e.small_count) {
                (Some(err), _) => Some(err.to_string()),
                (None, _) => None,
            };
            if error.is_some() {
                coverage += 1;
            }
            let id = i as i64 + 1;
            let parent: Option<i64> = if e.parent == u32::MAX { None } else { Some(e.parent as i64 + 1) };
            stmt.execute(params![
                id,
                parent,
                path,
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
                cls.category,
                cls.reason,
                error,
                cls.tag,
                cls.action,
                cls.cost,
            ])
            .map_err(|err| format!("{} ({})", err, path))?;
            if i % 100_000 == 0 && i > 0 {
                progress(Progress { entries: seen, dirs: dirs.load(Ordering::Relaxed), bytes, current: format!("{} of {} rows", i, entries.len()), phase: "write".into(), elapsed_ms: start.elapsed().as_millis() as u64 });
            }
        }
        drop(stmt);
        tx.execute("CREATE INDEX parents ON entries(parent, total DESC)", []).map_err(|e| e.to_string())?;
        tx.execute("CREATE INDEX totals ON entries(total)", []).map_err(|e| e.to_string())?;
        tx.execute("CREATE INDEX cats ON entries(category, total DESC)", []).map_err(|e| e.to_string())?;
        tx.execute("CREATE INDEX tags ON entries(tag, total DESC)", []).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        meta_put("complete", serde_json::json!(true))?;
        meta_put("finished", serde_json::json!(now_f64()))?;
        meta_put("entries", serde_json::json!(seen))?;
        meta_put("rows", serde_json::json!(entries.len()))?;
        meta_put("coverage_issues", serde_json::json!(coverage))?;
        meta_put("rules", serde_json::json!(crate::classify::RULES_VERSION))?;
        return Ok(ScanSummary {
            output: opts.output.clone(),
            entries: seen,
            rows: entries.len() as u64,
            bytes,
            elapsed_ms: start.elapsed().as_millis() as u64,
            coverage_issues: coverage,
        });
    }
}
