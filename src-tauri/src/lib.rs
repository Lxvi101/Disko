pub mod classify;
mod disk_access;
mod remnants;
mod reports;
mod scanner;
mod scan_storage;
mod xcode;
mod app_cleanup;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, State};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VolumeInfo {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileItem {
    pub id: Option<i64>,
    pub parent: Option<i64>,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub total: u64,
    pub allocated: u64,
    pub logical: u64,
    pub category: String,
    pub reason: String,
    pub tag: String,
    pub action: String,
    pub cost: String,
    pub tool: Option<String>,
    pub mtime_ns: Option<i64>,
    pub error: Option<String>,
    pub child_count: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub total: u64,
    pub category: String,
    pub reason: String,
    pub tag: String,
    pub action: String,
    pub children: Vec<TreeNode>,
    /// Bytes of children that were not returned because of the per-node limit.
    pub rest_total: u64,
    pub rest_count: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanMeta {
    pub root: String,
    pub started: Option<f64>,
    pub finished: Option<f64>,
    pub entries: Option<u64>,
    pub complete: bool,
    pub db_path: String,
    pub coverage_issues: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppInfo {
    pub home: String,
    pub workspace: String,
    pub db_path: String,
    pub db_loaded: bool,
    pub codex_path: Option<String>,
    pub codex_version: Option<String>,
    pub scan: Option<ScanMeta>,
    pub volume: VolumeInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OverviewData {
    pub root: String,
    pub volume: VolumeInfo,
    pub items: Vec<FileItem>,
    pub db_loaded: bool,
    pub entry_count: usize,
    pub category_totals: HashMap<String, u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CandidateItem {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub total: u64,
    pub category: String,
    pub reason: String,
    pub tag: String,
    pub action: String,
    pub cost: String,
    pub tool: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UnusedItem {
    pub path: String,
    pub name: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub days_inactive: u64,
    pub category: String,
    pub reason: String,
    pub last_used: Option<String>,
    pub last_accessed: Option<String>,
    pub modified: Option<String>,
    pub evidence: Option<String>,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexModel {
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub efforts: Vec<String>,
    pub default_effort: String,
    pub priority: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexModels {
    pub default_model: String,
    pub default_effort: String,
    pub models: Vec<CodexModel>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexEventPayload {
    pub request_id: String,
    pub event: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatContext {
    pub page: String,
    pub current_path: String,
    pub items: Vec<ContextItem>,
    pub staged: Vec<ContextItem>,
    pub history: Vec<HistoryTurn>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextItem {
    pub path: String,
    pub total: u64,
    pub category: String,
    pub kind: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub action: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryTurn {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexResult {
    pub request_id: String,
    pub success: bool,
    pub text: String,
    pub error: Option<String>,
    pub usage: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarantineInput {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarantineResult {
    pub journal_path: String,
    pub moved_items: Vec<String>,
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarantineEntry {
    pub original: String,
    pub quarantined: String,
    pub size: u64,
    pub present: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarantineJournal {
    pub id: String,
    pub path: String,
    pub created_at: u64,
    pub items: Vec<QuarantineEntry>,
    pub total_bytes: u64,
    pub purged: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanProgress {
    pub phase: String,
    pub message: String,
    pub entries: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub current: String,
    pub elapsed_ms: u64,
    pub done: bool,
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanInfo {
    pub path: String,
    pub file: String,
    pub root: String,
    pub started: Option<f64>,
    pub finished: Option<f64>,
    pub entries: Option<u64>,
    pub rows: Option<u64>,
    pub mode: String,
    pub complete: bool,
    pub size_bytes: u64,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ProcState {
    pub codex: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
    pub scan: Mutex<Option<Arc<Mutex<Child>>>>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn workspace_dir() -> PathBuf {
    if let Some(ws) = std::env::var_os("DISKO_WORKSPACE") {
        return PathBuf::from(ws);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(parent) = manifest.parent() {
        if parent.join("disko_cli").exists() {
            return parent.to_path_buf();
        }
    }
    home_dir().join(".disko")
}

fn reports_dir() -> PathBuf {
    let dir = workspace_dir().join("reports");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn active_file() -> PathBuf {
    reports_dir().join(".active")
}

fn db_path() -> PathBuf {
    let reports = reports_dir();
    if let Ok(name) = fs::read_to_string(active_file()) {
        let name = name.trim();
        if !name.is_empty() {
            let p = if name.starts_with('/') { PathBuf::from(name) } else { reports.join(name) };
            if p.exists() {
                return p;
            }
        }
    }
    reports.join("home.sqlite")
}


fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

fn resolve_from_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn codex_bin() -> Option<PathBuf> {
    let home = home_dir();
    first_existing(&[
        home.join(".codex/packages/standalone/current/bin/codex"),
        home.join(".local/bin/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
        home.join(".bun/bin/codex"),
        home.join(".npm-global/bin/codex"),
    ])
    .or_else(|| resolve_from_path("codex"))
}

pub(crate) fn volume_info(path: &Path) -> VolumeInfo {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    if let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        unsafe {
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                let total = stat.f_blocks as u64 * stat.f_frsize as u64;
                let available = stat.f_bavail as u64 * stat.f_frsize as u64;
                return VolumeInfo {
                    total,
                    available,
                    used: total.saturating_sub(available),
                };
            }
        }
    }
    VolumeInfo {
        total: 0,
        available: 0,
        used: 0,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

// ---------------------------------------------------------------------------
// Classification (mirrors disko_cli.core.classify for live fallbacks)
// ---------------------------------------------------------------------------

pub(crate) fn classify_path(path: &str, home: &str) -> (String, String) {
    let c = classify::classify(path, home);
    (c.category.to_string(), c.reason)
}

fn home_str() -> &'static str {
    static HOME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOME.get_or_init(|| home_dir().to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// SQLite access
// ---------------------------------------------------------------------------

fn open_db() -> Option<Connection> {
    let p = db_path();
    if !p.exists() {
        return None;
    }
    Connection::open_with_flags(&p, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

const ITEM_COLUMNS: &str = "id, parent, path, kind, total, allocated, logical, category, reason, mtime_ns, error";

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileItem> {
    let path: String = row.get(2)?;
    let name = file_name_of(&path);
    // Rules are applied live so a classifier update shows up without a rescan.
    let c = classify::classify(&path, home_str());
    Ok(FileItem {
        id: row.get(0)?,
        parent: row.get(1)?,
        path,
        name,
        kind: row.get(3)?,
        total: row.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0) as u64,
        allocated: row.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0) as u64,
        logical: row.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u64,
        category: c.category.to_string(),
        reason: c.reason,
        tag: c.tag.to_string(),
        action: c.action.to_string(),
        cost: c.cost.to_string(),
        tool: c.tool,
        mtime_ns: row.get(9)?,
        error: row.get(10)?,
        child_count: None,
    })
}

fn children_of(conn: &Connection, parent_id: i64, limit: usize) -> Result<Vec<FileItem>, String> {
    let sql = format!(
        "SELECT {} FROM entries WHERE parent = ?1 ORDER BY total DESC LIMIT ?2",
        ITEM_COLUMNS
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![parent_id, limit as i64], row_to_item)
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn id_for_path(conn: &Connection, path: &str) -> Option<i64> {
    conn.query_row("SELECT id FROM entries WHERE path = ?1", [path], |r| r.get(0))
        .ok()
}

fn item_for_path(conn: &Connection, path: &str) -> Option<FileItem> {
    let sql = format!("SELECT {} FROM entries WHERE path = ?1", ITEM_COLUMNS);
    conn.query_row(&sql, [path], row_to_item).ok()
}

// ---------------------------------------------------------------------------
// Keeping the scan in step with quarantine moves
// ---------------------------------------------------------------------------

fn open_db_rw() -> Option<Connection> {
    let p = db_path();
    if !p.exists() {
        return None;
    }
    Connection::open(&p).ok()
}

fn refresh_row_count(tx: &Connection) {
    let _ = tx.execute("UPDATE meta SET value = (SELECT count(*) FROM entries) WHERE key = 'rows'", []);
}

fn adjust_ancestors(tx: &Connection, mut parent: Option<i64>, total: i64, allocated: i64, logical: i64) -> Result<(), String> {
    while let Some(pid) = parent {
        tx.execute(
            "UPDATE entries SET total = max(0, coalesce(total, 0) + ?1), allocated = max(0, coalesce(allocated, 0) + ?2), logical = max(0, coalesce(logical, 0) + ?3) WHERE id = ?4",
            rusqlite::params![total, allocated, logical, pid],
        )
        .map_err(|e| e.to_string())?;
        parent = tx
            .query_row("SELECT parent FROM entries WHERE id = ?1", [pid], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten();
    }
    Ok(())
}

/// Drop paths (and everything under them) from the active scan and shrink every ancestor's totals,
/// so the sunburst, lists and overview reflect a deletion without a rescan.
fn detach_scan_entries(paths: &[String]) -> Result<usize, String> {
    if paths.is_empty() {
        return Ok(0);
    }
    let Some(mut conn) = open_db_rw() else { return Ok(0) };
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut removed = 0usize;
    for path in paths {
        let Some(item) = item_for_path(&tx, path) else { continue };
        // '0' is the character after '/', so this range is exactly "path/…" and uses the path index.
        let lo = format!("{}/", path);
        let hi = format!("{}0", path);
        tx.execute("DELETE FROM entries WHERE path = ?1 OR (path >= ?2 AND path < ?3)", rusqlite::params![path, lo, hi])
            .map_err(|e| e.to_string())?;
        adjust_ancestors(&tx, item.parent, -(item.total as i64), -(item.allocated as i64), -(item.logical as i64))?;
        removed += 1;
    }
    refresh_row_count(&tx);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(removed)
}

/// Put restored items back into the scan as single entries at their journaled size. Their contents
/// are not re-walked, so a restored folder shows as one slice until the next scan.
fn reattach_scan_entries(items: &[(String, u64)]) -> Result<usize, String> {
    if items.is_empty() {
        return Ok(0);
    }
    let Some(mut conn) = open_db_rw() else { return Ok(0) };
    let home = home_dir().to_string_lossy().to_string();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut added = 0usize;
    for (path, size) in items {
        if item_for_path(&tx, path).is_some() {
            continue;
        }
        let Some(parent_path) = Path::new(path).parent().map(|p| p.to_string_lossy().to_string()) else { continue };
        let Some(parent_id) = id_for_path(&tx, &parent_path) else { continue };
        let meta = fs::symlink_metadata(path).ok();
        let kind = if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) { "directory" } else { "file" };
        let mtime_ns: Option<i64> = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64);
        let c = classify::classify(path, &home);
        let size = *size as i64;
        tx.execute(
            "INSERT INTO entries(parent, path, kind, size, mtime_ns, allocated, total, logical, category, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6, ?7, ?8)",
            rusqlite::params![parent_id, path, kind, size, mtime_ns, size, c.category, c.reason],
        )
        .map_err(|e| e.to_string())?;
        adjust_ancestors(&tx, Some(parent_id), size, size, size)?;
        added += 1;
    }
    refresh_row_count(&tx);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(added)
}

fn scan_meta(conn: &Connection) -> Option<ScanMeta> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta").ok()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()?;
    let mut map: HashMap<String, serde_json::Value> = HashMap::new();
    for (k, v) in rows.flatten() {
        if let Ok(val) = serde_json::from_str(&v) {
            map.insert(k, val);
        }
    }
    let coverage: u64 = 0; // expensive on large scans; see get_coverage_issue_count
    Some(ScanMeta {
        root: map.get("root").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        started: map.get("started").and_then(|v| v.as_f64()),
        finished: map.get("finished").and_then(|v| v.as_f64()),
        entries: map.get("entries").and_then(|v| v.as_u64()),
        complete: map.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
        db_path: db_path().to_string_lossy().to_string(),
        coverage_issues: coverage,
    })
}

fn live_children(dir: &Path, home: &str) -> Vec<FileItem> {
    let mut items = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let p_str = p.to_string_lossy().to_string();
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let c = classify::classify(&p_str, home);
            items.push(FileItem {
                id: None,
                parent: None,
                path: p_str,
                name,
                kind: if is_dir { "directory".into() } else { "file".into() },
                total: if is_dir { 0 } else { size },
                allocated: size,
                logical: size,
                category: c.category.to_string(),
                reason: c.reason,
                tag: c.tag.to_string(),
                action: c.action.to_string(),
                cost: c.cost.to_string(),
                tool: c.tool,
                mtime_ns: None,
                error: None,
                child_count: None,
            });
        }
    }
    items.sort_by(|a, b| b.total.cmp(&a.total));
    items
}

// ---------------------------------------------------------------------------
// Commands: info & browsing
// ---------------------------------------------------------------------------

#[tauri::command]
async fn get_app_info(app: AppHandle, state: State<'_, ProcState>) -> Result<AppInfo, String> {
    // A scan written by an older rule set carries stale verdicts; bring it up to date in the
    // background so a rule change lands without a full rescan. Never while a scan is writing.
    if state.scan.lock().await.is_none() {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || match reclassify_scan_sync() {
            Ok(0) => {}
            Ok(n) => {
                let _ = handle.emit("scan-reclassified", n);
            }
            Err(e) => eprintln!("reclassify after rule change failed: {}", e),
        });
    }
    let home = home_dir();
    let codex_path = codex_bin();
    let codex_version = if let Some(bin) = &codex_path {
        Command::new(bin)
            .arg("--version")
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    };
    let scan = open_db().and_then(|c| scan_meta(&c));
    Ok(AppInfo {
        home: home.to_string_lossy().to_string(),
        workspace: workspace_dir().to_string_lossy().to_string(),
        db_path: db_path().to_string_lossy().to_string(),
        db_loaded: db_path().exists(),
        codex_path: codex_path.map(|p| p.to_string_lossy().to_string()),
        codex_version,
        scan,
        volume: volume_info(&home),
    })
}

fn load_disk_overview_sync() -> Result<OverviewData, String> {
    let home = home_dir();
    let home_str = home.to_string_lossy().to_string();
    let volume = volume_info(&home);

    if let Some(conn) = open_db() {
        let root_path: String = conn
            .query_row("SELECT path FROM entries WHERE parent IS NULL LIMIT 1", [], |r| r.get(0))
            .unwrap_or_else(|_| home_str.clone());
        let root_id = id_for_path(&conn, &root_path).unwrap_or(1);
        let items = children_of(&conn, root_id, 80)?;
        let count: i64 = conn
            .query_row("SELECT value FROM meta WHERE key='rows'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .or_else(|| conn.query_row("SELECT count(*) FROM entries", [], |r| r.get(0)).ok())
            .unwrap_or(0);
        let mut category_totals = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT category, SUM(total) FROM entries WHERE parent = ?1 GROUP BY category") {
            if let Ok(rows) = stmt.query_map([root_id], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?))) {
                for (cat, sum) in rows.flatten() {
                    category_totals.insert(cat.unwrap_or_else(|| "review".into()), sum.unwrap_or(0).max(0) as u64);
                }
            }
        }
        return Ok(OverviewData {
            root: root_path,
            volume,
            items,
            db_loaded: true,
            entry_count: count as usize,
            category_totals,
        });
    }

    let items = live_children(&home, &home_str);
    Ok(OverviewData {
        root: home_str,
        volume,
        items,
        db_loaded: false,
        entry_count: 0,
        category_totals: HashMap::new(),
    })
}

fn query_folder_children_sync(path: String, limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    let home_str = home_dir().to_string_lossy().to_string();
    let limit = limit.unwrap_or(200);
    if let Some(conn) = open_db() {
        if let Some(id) = id_for_path(&conn, &path) {
            return children_of(&conn, id, limit);
        }
    }
    let p = Path::new(&path);
    if !p.is_dir() {
        return Ok(Vec::new());
    }
    Ok(live_children(p, &home_str))
}

fn build_tree(conn: &Connection, item: &FileItem, depth: usize, limits: &[usize], min_total: u64) -> TreeNode {
    let mut node = TreeNode {
        path: item.path.clone(),
        name: item.name.clone(),
        kind: item.kind.clone(),
        total: item.total,
        category: item.category.clone(),
        reason: item.reason.clone(),
        tag: item.tag.clone(),
        action: item.action.clone(),
        children: Vec::new(),
        rest_total: 0,
        rest_count: 0,
    };
    if depth == 0 || item.kind != "directory" {
        return node;
    }
    let Some(id) = item.id else { return node };
    let limit = *limits.first().unwrap_or(&8);
    let Ok(children) = children_of(conn, id, limit + 1) else { return node };
    let shown: Vec<&FileItem> = children.iter().take(limit).filter(|c| c.total >= min_total).collect();
    let shown_total: u64 = shown.iter().map(|c| c.total).sum();
    if children.len() > shown.len() {
        let (cnt, sum): (i64, Option<i64>) = conn
            .query_row(
                "SELECT count(*), SUM(total) FROM entries WHERE parent = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((children.len() as i64, Some(shown_total as i64)));
        node.rest_count = (cnt as u64).saturating_sub(shown.len() as u64);
        node.rest_total = (sum.unwrap_or(0).max(0) as u64).saturating_sub(shown_total);
    }
    let next_limits = if limits.len() > 1 { &limits[1..] } else { limits };
    for c in shown {
        if c.total == 0 {
            continue;
        }
        node.children.push(build_tree(conn, c, depth - 1, next_limits, min_total));
    }
    node
}

fn get_subtree_sync(path: String, depth: Option<usize>) -> Result<TreeNode, String> {
    let depth = depth.unwrap_or(5).min(9);
    let conn = open_db().ok_or("No scan database loaded")?;
    let item = item_for_path(&conn, &path).ok_or("Path not found in scan")?;
    // Anything under 0.15% of the current folder is folded into the grey remainder.
    let min_total = (item.total as f64 * 0.0015) as u64;
    let limits = [64usize, 40, 28, 20, 14, 10, 8, 8, 8];
    Ok(build_tree(&conn, &item, depth, &limits, min_total.max(1)))
}

fn get_path_info_sync(path: String) -> Result<Option<FileItem>, String> {
    let Some(conn) = open_db() else { return Ok(None) };
    let mut item = item_for_path(&conn, &path);
    if let Some(it) = item.as_mut() {
        if let Some(id) = it.id {
            let cnt: i64 = conn
                .query_row("SELECT count(*) FROM entries WHERE parent = ?1", [id], |r| r.get(0))
                .unwrap_or(0);
            it.child_count = Some(cnt as u64);
        }
    }
    Ok(item)
}

fn search_entries_sync(query: String, limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    let q = query.trim();
    if q.len() < 2 {
        return Ok(Vec::new());
    }
    let conn = open_db().ok_or("No scan database loaded")?;
    let sql = format!(
        "SELECT {} FROM entries WHERE path LIKE ?1 ESCAPE '\\' AND total > 0 ORDER BY total DESC LIMIT ?2",
        ITEM_COLUMNS
    );
    let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let pattern = format!("%{}%", escaped);
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![pattern, limit.unwrap_or(40) as i64], row_to_item)
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn get_largest_files_sync(limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    let conn = open_db().ok_or("No scan database loaded")?;
    let sql = format!(
        "SELECT {} FROM entries WHERE kind = 'file' ORDER BY total DESC LIMIT ?1",
        ITEM_COLUMNS
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.unwrap_or(40) as i64], row_to_item)
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

// ---------------------------------------------------------------------------
// Commands: report JSON files
// ---------------------------------------------------------------------------

fn read_json(name: &str) -> Option<serde_json::Value> {
    let p = reports_dir().join(name);
    let content = fs::read_to_string(p).ok()?;
    serde_json::from_str(&content).ok()
}

fn get_candidates_sync() -> Result<Vec<CandidateItem>, String> {
    let mut json = read_json("candidates.json");
    let stale = json.as_ref().map(|j| j.get("rules").and_then(|v| v.as_u64()).unwrap_or(0) != classify::RULES_VERSION as u64).unwrap_or(true);
    if stale {
        let db = db_path();
        if db.exists() {
            let _ = reports::write_candidates(&db, &reports_dir().join("candidates.json"), &home_dir(), 80);
            json = read_json("candidates.json");
        }
    }
    let Some(json) = json else { return Ok(Vec::new()) };
    let mut res = Vec::new();
    if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
        for it in items {
            let path = it.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if path.is_empty() {
                continue;
            }
            let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(|v| v.to_string());
            res.push(CandidateItem {
                name: file_name_of(&path),
                path,
                kind: s("kind").unwrap_or_else(|| "directory".into()),
                total: it.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
                category: s("category").unwrap_or_else(|| "review".into()),
                reason: s("reason").unwrap_or_default(),
                tag: s("tag").unwrap_or_default(),
                action: s("action").unwrap_or_else(|| "quarantine".into()),
                cost: s("cost").unwrap_or_else(|| "unknown".into()),
                tool: s("tool"),
            });
        }
    }
    Ok(res)
}

fn parse_unused(json: serde_json::Value, kind: &str) -> Vec<UnusedItem> {
    let mut res = Vec::new();
    if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
        for it in items {
            let path = it.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if path.is_empty() {
                continue;
            }
            let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
            res.push(UnusedItem {
                name: file_name_of(&path),
                path,
                logical_bytes: it.get("logical_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                allocated_bytes: it.get("allocated_bytes_estimate").and_then(|v| v.as_u64()).unwrap_or(0),
                days_inactive: it.get("days_since_activity_evidence").and_then(|v| v.as_u64()).unwrap_or(0),
                category: s("category").unwrap_or_else(|| "review".into()),
                reason: s("policy_reason").unwrap_or_default(),
                last_used: s("last_used"),
                last_accessed: s("last_accessed"),
                modified: s("modified"),
                evidence: s("evidence"),
                kind: kind.to_string(),
            });
        }
    }
    res
}

fn get_unused_files_sync() -> Result<Vec<UnusedItem>, String> {
    Ok(read_json("unused-files.json").map(|j| parse_unused(j, "file")).unwrap_or_default())
}

fn get_unused_apps_sync() -> Result<Vec<UnusedItem>, String> {
    Ok(read_json("unused-apps.json").map(|j| parse_unused(j, "app")).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Commands: Codex
// ---------------------------------------------------------------------------

fn config_default_model() -> (Option<String>, Option<String>) {
    let cfg = fs::read_to_string(home_dir().join(".codex/config.toml")).unwrap_or_default();
    let mut model = None;
    let mut effort = None;
    for line in cfg.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            break; // only top-level keys
        }
        if let Some(rest) = l.strip_prefix("model") {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                model = Some(v.trim().trim_matches('"').to_string());
            }
        }
        if let Some(rest) = l.strip_prefix("model_reasoning_effort") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                effort = Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    (model, effort)
}

fn list_codex_models_sync() -> Result<CodexModels, String> {
    let (cfg_model, cfg_effort) = config_default_model();
    let mut models: Vec<CodexModel> = Vec::new();
    if let Ok(content) = fs::read_to_string(home_dir().join(".codex/models_cache.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(arr) = json.get("models").and_then(|v| v.as_array()) {
                for m in arr {
                    let visibility = m.get("visibility").and_then(|v| v.as_str()).unwrap_or("list");
                    if visibility != "list" {
                        continue;
                    }
                    let slug = m.get("slug").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if slug.is_empty() {
                        continue;
                    }
                    let efforts: Vec<String> = m
                        .get("supported_reasoning_levels")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|e| e.get("effort").and_then(|v| v.as_str()).map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_else(|| vec!["low".into(), "medium".into(), "high".into()]);
                    models.push(CodexModel {
                        display_name: m.get("display_name").and_then(|v| v.as_str()).unwrap_or(&slug).to_string(),
                        description: m.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        default_effort: m.get("default_reasoning_level").and_then(|v| v.as_str()).unwrap_or("medium").to_string(),
                        priority: m.get("priority").and_then(|v| v.as_i64()).unwrap_or(999),
                        efforts,
                        slug,
                    });
                }
            }
        }
    }
    models.sort_by_key(|m| m.priority);
    if models.is_empty() {
        for (slug, name, desc) in [
            ("gpt-6-astra", "GPT-6-Astra", "Most capable model for complex, demanding work."),
            ("gpt-5.6-sol", "GPT-5.6-Sol", "Reliable agentic workhorse for everyday tasks."),
            ("gpt-5.6-terra", "GPT-5.6-Terra", "Balanced agentic model for everyday work."),
            ("gpt-5.5", "GPT-5.5", "Previous-generation model."),
        ] {
            models.push(CodexModel {
                slug: slug.into(),
                display_name: name.into(),
                description: desc.into(),
                efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
                default_effort: "medium".into(),
                priority: 0,
            });
        }
    }
    let default_model = cfg_model
        .filter(|m| models.iter().any(|x| &x.slug == m))
        .unwrap_or_else(|| models[0].slug.clone());
    let default_effort = cfg_effort.unwrap_or_else(|| {
        models
            .iter()
            .find(|m| m.slug == default_model)
            .map(|m| m.default_effort.clone())
            .unwrap_or_else(|| "medium".into())
    });
    Ok(CodexModels {
        default_model,
        default_effort,
        models,
    })
}

fn fmt_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", b, units[i])
    } else {
        format!("{:.1} {}", v, units[i])
    }
}

fn build_prompt(prompt: &str, ctx: &ChatContext) -> String {
    let ws = workspace_dir();
    let home = home_dir();
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "disko".into());
    let reports = reports_dir();
    let mut s = String::new();
    s.push_str("# Role\n");
    s.push_str("You are Disko, the assistant inside a macOS disk-space visualizer and cleanup app. ");
    s.push_str("You help the user understand where their storage went and decide what is safe to remove. ");
    s.push_str("Be concise, concrete, and honest about uncertainty. Use short paragraphs and bullet lists; avoid headers unless the answer is long. ");
    s.push_str("The user can choose recoverable quarantine or permanent deletion in the cleanup dialog. Quarantine does not reclaim space until purged. Never claim deletion or reclaimed space without a successful cleanup result.\n\n");

    s.push_str("# Tools available to you\n");
    let db = db_path();
    let db_str = db.to_string_lossy().to_string();
    let (scan_root, scan_mode, scan_rules) = open_db()
        .map(|c| {
            let get = |k: &str| c.query_row("SELECT value FROM meta WHERE key=?1", [k], |r| r.get::<_, String>(0)).ok().map(|v| v.trim_matches('"').to_string());
            (get("root").unwrap_or_default(), get("mode").unwrap_or_else(|| "thorough".into()), get("rules").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0))
        })
        .unwrap_or_default();
    s.push_str(&format!(
        "You run in a read-only sandbox with the working directory `{}`. You may run shell commands to inspect the filesystem (ls, du -sh, find, stat, mdls) and read-only manager inventories (see below).\n",
        ws.display()
    ));
    s.push_str(&format!(
        "A completed metadata scan of `{}` is available at `{}` ({} mode{}). Use it for fast, exact answers instead of slow `du` walks:\n",
        if scan_root.is_empty() { home.to_string_lossy().to_string() } else { scan_root.clone() },
        db_str,
        scan_mode,
        if scan_mode == "quick" { ": files under 256 KB are folded into their parent folder's total and have no row of their own" } else { "" }
    ));
    s.push_str(&format!("- Immediate children of a folder, largest first: `sqlite3 -json \"{}\" \"SELECT path,kind,total,category,tag FROM entries WHERE parent=(SELECT id FROM entries WHERE path='/absolute/path') ORDER BY total DESC LIMIT 40\"`\n", db_str));
    s.push_str(&format!("- Largest files anywhere: `sqlite3 -json \"{}\" \"SELECT path,total,category FROM entries WHERE kind='file' ORDER BY total DESC LIMIT 40\"`\n", db_str));
    s.push_str(&format!("- Everything a rule tagged, e.g. manager-owned stores: `sqlite3 -json \"{}\" \"SELECT path,total,tool FROM entries WHERE tag='managed-cleanup' AND kind='directory' ORDER BY total DESC LIMIT 60\"`\n", db_str));
    s.push_str(&format!("- Find by name: `sqlite3 -json \"{}\" \"SELECT path,total FROM entries WHERE path LIKE '%node_modules' ORDER BY total DESC LIMIT 40\"`\n", db_str));
    if scan_rules != classify::RULES_VERSION {
        let ver = if scan_rules == 0 { String::new() } else { format!(" (v{})", scan_rules) };
        s.push_str(&format!("- This scan was written by an older rule set{}; its `category`/`reason` columns may be stale and it has no `tag` column. Classify paths live instead: `\"{}\" --classify /path/one /path/two`.\n", ver, exe));
    } else {
        s.push_str(&format!("- Live classification of any path (category, tag, action, recreate cost, tool command): `\"{}\" --classify /path/one /path/two`.\n", exe));
    }
    s.push_str(&format!("- Ready-made reports in `{}`: `candidates.json` (ranked cleanup candidates with tag/action/tool), `unused-files.json` (large files with no recent activity), `remnants.json` (leftovers of apps that are no longer installed, grouped per bundle id with a confidence score, plus launch agents whose program is gone).\n", reports.display()));
    s.push_str("- Read-only manager inventories you may run when relevant: `docker system df -v`, `brew cleanup --dry-run`, `pnpm store path`, `xcrun simctl list devices unavailable`, `uv cache dir`, `go env GOCACHE GOMODCACHE`, `rustup toolchain list`, `nvm ls` (via `source ~/.nvm/nvm.sh`), `ollama list`, `hf cache scan`, `conda clean --all --dry-run`, `tmutil listlocalsnapshots /`.\n");
    s.push_str("`total` is bytes including descendants; never sum a parent with its children.\n\n");

    s.push_str("# Taxonomy\n");
    s.push_str("Every path has a base category answering \"can this be proposed at all?\": rebuildable = known cache or generated output; review = user data or manager-owned state that needs a human decision; keep = app state, profiles, repositories, use the owning app; protected = system, credentials, cloud-managed, outside home, never touched.\n");
    s.push_str("Orthogonal to that, a tag names the class (app-cache, browser-cache, browser-profile, build-output, dependency-cache, managed-cleanup, managed-container-store, unused-runtime, generated-project-output, generated-environment, model-cache, downloaded-model, old-backup, old-installer, release-artifact, diagnostic, orphaned-app, shared-app-state, tcc-gated, system-managed, ...), an action says how to remove it (quarantine = move into Disko's quarantine; manager_command = the owning tool has safer garbage collection, its command is in `tool`; owner_app = delete inside the owning application; report_only = explain, do not offer removal), and a recreate cost (instant, local-rebuild, network-small, network-large, unknown, irreplaceable).\n\n");

    s.push_str("# Decision model\n");
    s.push_str("1. Clearly disposable: the owner or manager itself says the object is cache or unreferenced, or it is generated output whose inputs are preserved (DerivedData, node_modules with a lockfile, pip/uv/pnpm caches, Go build cache, Docker build cache, unavailable simulators). Propose freely.\n");
    s.push_str("2. Likely abandoned: an exact bundle id is gone from every installed app, nothing of it is running, and the leftovers are stale. Use remnants.json; cache members can be preselected, persistent state needs confirmation.\n");
    s.push_str("3. Needs confirmation: device backups, unused runtimes and toolchains, browser profiles, Application Support data, VM disks, model weights, Xcode archives, Docker containers, old applications, downloadable Apple content. Present with the recreate cost and let the user decide.\n");
    s.push_str("4. Do not touch: credentials, cloud-managed roots, active browser profiles, Mail/Messages/Photos data, Xcode UserData, shared Group Containers without complete ownership proof, /System, /private/var/folders, /private/var/vm, Time Machine local snapshots (macOS already counts them as available), and Docker volumes.\n");
    s.push_str("Rules of thumb: prefer the manager's command over moving its store (docker system prune, brew cleanup, pnpm store prune, uv cache prune, go clean -modcache, conda clean --all, rustup toolchain uninstall, nvm uninstall, ollama rm, hf cache delete, xcrun simctl delete unavailable). Age alone never proves abandonment; kMDItemLastUsedDate only updates on Launch Services opens, so treat it as one vote. Never call a browser profile, Maven .m2, Docker volume, Xcode archive, or JetBrains cache root (it holds Local History) rebuildable. Mention rebuild time or download size when it matters, and quote sizes from the scan rather than guessing.\n\n");

    s.push_str("# Response format\n");
    s.push_str("Write the answer in Markdown. When you recommend specific items, finish with a fenced block tagged `disko-actions` containing JSON like:\n");
    s.push_str("```disko-actions\n{\"suggestions\":[{\"path\":\"/absolute/path\",\"bytes\":123456,\"reason\":\"why it is safe\",\"confidence\":\"high\",\"action\":\"quarantine\"},{\"path\":\"/Users/x/.cache/uv\",\"bytes\":1200000,\"reason\":\"uv tracks what is still referenced\",\"confidence\":\"high\",\"action\":\"command\",\"command\":\"uv cache prune\"}]}\n```\n");
    s.push_str("Only include absolute paths you verified exist. `action` is `quarantine` (the app can collect it) or `command` (show the manager command instead; the app will not move that path). Use confidence high/medium/low. Omit the block when you have nothing to recommend.\n\n");

    s.push_str("# Current app context\n");
    s.push_str(&format!("- Page: {}\n", ctx.page));
    if !ctx.current_path.is_empty() {
        s.push_str(&format!("- Folder in view: {}\n", ctx.current_path));
    }
    if !ctx.items.is_empty() {
        s.push_str("- Largest items in view:\n");
        for it in ctx.items.iter().take(25) {
            let tag = if it.tag.is_empty() { String::new() } else { format!(" {}", it.tag) };
            s.push_str(&format!("  - {} — {} [{}{}] ({})\n", it.path, fmt_bytes(it.total), it.category, tag, it.kind));
        }
    }
    if !ctx.staged.is_empty() {
        s.push_str("- Items the user already collected for cleanup:\n");
        for it in ctx.staged.iter().take(40) {
            let tag = if it.tag.is_empty() { String::new() } else { format!(" {}", it.tag) };
            let act = if it.action.is_empty() || it.action == "quarantine" { String::new() } else { format!(", action {}", it.action) };
            s.push_str(&format!("  - {} — {} [{}{}{}]\n", it.path, fmt_bytes(it.total), it.category, tag, act));
        }
    }
    s.push('\n');
    if !ctx.history.is_empty() {
        s.push_str("# Conversation so far\n");
        for turn in ctx.history.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            let who = if turn.role == "user" { "User" } else { "Disko" };
            let text: String = turn.text.chars().take(2500).collect();
            s.push_str(&format!("{}: {}\n\n", who, text));
        }
    }
    s.push_str("# New user message\n");
    s.push_str(prompt);
    s.push('\n');
    s
}

fn is_noise(msg: &str) -> bool {
    msg.contains("hook-trust")
        || msg.contains("Model metadata for")
        || msg.contains("models cache")
        || msg.contains("Reading additional input")
}

#[tauri::command]
async fn codex_chat(
    app: AppHandle,
    state: State<'_, ProcState>,
    request_id: String,
    prompt: String,
    model: Option<String>,
    effort: Option<String>,
    context: ChatContext,
) -> Result<CodexResult, String> {
    let bin = codex_bin().ok_or("Codex CLI not found. Install it from https://chatgpt.com/codex or run `codex update`.")?;
    let full_prompt = build_prompt(&prompt, &context);
    let ws = workspace_dir();

    let mut cmd = Command::new(&bin);
    cmd.arg("exec")
        .arg("--json")
        .arg("--ephemeral")
        .arg("--skip-git-repo-check")
        .arg("-s")
        .arg("read-only")
        .arg("--color")
        .arg("never")
        .arg("-C")
        .arg(&ws);
    if let Some(m) = model.as_ref().filter(|m| !m.is_empty()) {
        cmd.arg("-m").arg(m);
    }
    if let Some(e) = effort.as_ref().filter(|e| !e.is_empty()) {
        cmd.arg("-c").arg(format!("model_reasoning_effort=\"{}\"", e));
    }
    cmd.arg("-")
        .current_dir(&ws)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("Failed to start Codex: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(full_prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let stdout = child.stdout.take().ok_or("No stdout from Codex")?;
    let stderr = child.stderr.take();

    let child = Arc::new(Mutex::new(child));
    state.codex.lock().await.insert(request_id.clone(), child.clone());

    // Drain stderr in the background so the process never blocks on a full pipe.
    let stderr_task = tokio::spawn(async move {
        let mut collected = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.contains(" ERROR ") && !line.contains(" WARN ") && !line.trim().is_empty() {
                    collected.push_str(&line);
                    collected.push('\n');
                }
            }
        }
        collected
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut final_text = String::new();
    let mut messages: Vec<String> = Vec::new();
    let mut error: Option<String> = None;
    let mut usage: Option<serde_json::Value> = None;

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(trimmed) else { continue };
        let ty = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "item.completed" | "item.started" | "item.updated" => {
                let item = ev.get("item").cloned().unwrap_or(serde_json::Value::Null);
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if item_type == "error" {
                    let msg = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    if is_noise(msg) {
                        continue;
                    }
                }
                if ty == "item.completed" && item_type == "agent_message" {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        messages.push(t.to_string());
                    }
                }
            }
            "turn.completed" => {
                usage = ev.get("usage").cloned();
            }
            "turn.failed" | "error" => {
                let msg = ev
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| ev.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("Codex turn failed");
                error = Some(friendly_error(msg));
            }
            _ => {}
        }
        let _ = app.emit(
            "codex-event",
            CodexEventPayload {
                request_id: request_id.clone(),
                event: ev,
            },
        );
    }

    let status = { child.lock().await.wait().await.ok() };
    state.codex.lock().await.remove(&request_id);
    let stderr_text = stderr_task.await.unwrap_or_default();

    if !messages.is_empty() {
        final_text = messages.join("\n\n");
    }
    let success = error.is_none() && status.map(|s| s.success()).unwrap_or(false);
    if !success && error.is_none() {
        let code = status.and_then(|s| s.code());
        error = Some(if let Some(c) = code {
            if stderr_text.trim().is_empty() {
                if c == 130 || c == 143 || c == 137 {
                    "Stopped.".to_string()
                } else {
                    format!("Codex exited with status {}", c)
                }
            } else {
                stderr_text.trim().chars().take(600).collect()
            }
        } else {
            "Stopped.".to_string()
        });
    }

    Ok(CodexResult {
        request_id,
        success,
        text: final_text,
        error,
        usage,
    })
}

fn friendly_error(raw: &str) -> String {
    // The CLI often wraps the API error as a JSON string.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(m) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    raw.to_string()
}

#[tauri::command]
async fn codex_cancel(state: State<'_, ProcState>, request_id: String) -> Result<bool, String> {
    let child = state.codex.lock().await.get(&request_id).cloned();
    if let Some(c) = child {
        let mut guard = c.lock().await;
        let _ = guard.start_kill();
        return Ok(true);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Commands: scanning
// ---------------------------------------------------------------------------

fn emit_scan(app: &AppHandle, p: ScanProgress) {
    let _ = app.emit("scan-progress", p);
}

fn simple_progress(phase: &str, message: &str, done: bool, ok: bool) -> ScanProgress {
    ScanProgress { phase: phase.into(), message: message.into(), entries: 0, dirs: 0, bytes: 0, current: String::new(), elapsed_ms: 0, done, ok }
}

fn slug(root: &str) -> String {
    if root == "/" {
        return "disk".into();
    }
    let name = Path::new(root).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "scan".into());
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn get_remnants_sync(force: bool) -> Result<remnants::RemnantsReport, String> {
    let out = remnants::report_path(&reports_dir());
    let db = db_path();
    let db_opt = db.exists().then_some(db.as_path());
    if !force {
        if let Some(r) = remnants::read(&out) {
            let same_db = db_opt.map(|d| d.to_string_lossy() == r.db).unwrap_or(r.db.is_empty());
            if r.rules == classify::RULES_VERSION && same_db {
                return Ok(r);
            }
        }
    }
    remnants::write(db_opt, &out, &home_dir())
}

/// `disko --classify PATH...`: print the live classification of paths as JSON.
pub fn classify_cli(args: Vec<String>) -> i32 {
    let i = args.iter().position(|a| a == "--classify").unwrap_or(0);
    let paths: Vec<&String> = args.iter().skip(i + 1).filter(|a| !a.starts_with("--")).collect();
    if paths.is_empty() {
        eprintln!("usage: disko --classify PATH [PATH...]");
        return 2;
    }
    let home = home_dir().to_string_lossy().to_string();
    let rows: Vec<serde_json::Value> = paths
        .iter()
        .map(|p| {
            let abs = if p.starts_with('/') { (*p).clone() } else if let Some(rest) = p.strip_prefix("~/") { format!("{}/{}", home, rest) } else { format!("{}/{}", std::env::current_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(), p) };
            let c = classify::classify(&abs, &home);
            serde_json::json!({ "path": abs, "category": c.category, "tag": c.tag, "tag_label": classify::tag_label(c.tag), "action": c.action, "recreate_cost": c.cost, "tool": c.tool, "reason": c.reason })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
    0
}

/// `disko --remnants [DB] [--out DIR]`: rebuild remnants.json (uninstalled-app leftovers).
pub fn remnants_cli(args: Vec<String>) -> i32 {
    let i = args.iter().position(|a| a == "--remnants").unwrap_or(0);
    let db = args.get(i + 1).filter(|a| !a.starts_with("--")).map(PathBuf::from).unwrap_or_else(db_path);
    let out_dir = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(reports_dir);
    let db_opt = db.exists().then_some(db.as_path());
    match remnants::write(db_opt, &remnants::report_path(&out_dir), &home_dir()) {
        Ok(r) => {
            println!("{}", serde_json::json!({ "groups": r.groups.len(), "agents": r.agents.len(), "installed_apps": r.installed_apps, "bytes": r.groups.iter().map(|g| g.total).sum::<u64>(), "out": remnants::report_path(&out_dir) }));
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

/// `disko --reports DB [--out DIR]`: regenerate candidates.json and unused-files.json for a scan.
pub fn reports_cli(args: Vec<String>) -> i32 {
    let db = args.iter().position(|a| a == "--reports").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(reports_dir);
    let Some(db) = db else {
        eprintln!("usage: disko --reports DB [--out DIR]");
        return 2;
    };
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let t = std::time::Instant::now();
    let c = reports::write_candidates(&db, &out.join("candidates.json"), &home_dir(), 80);
    let t1 = t.elapsed().as_millis();
    let u = rt.block_on(reports::write_unused(db.clone(), out.join("unused-files.json"), 90, 100 * 1024 * 1024, 80));
    let r = remnants::write(Some(&db), &remnants::report_path(&out), &home_dir()).map(|r| r.groups.len());
    println!("{}", serde_json::json!({ "candidates": c.as_ref().ok(), "candidates_ms": t1, "unused": u.as_ref().ok(), "remnants": r.as_ref().ok(), "total_ms": t.elapsed().as_millis(), "errors": [c.err(), u.err(), r.err()] }));
    0
}

/// Headless entry point: `disko --scan ROOT --output DB [--quick] [--exclude P]... [--progress-file F] [--cancel-file F]`.
/// Prints JSON progress lines to stdout. Used for normal scans (child process) and admin scans (via osascript).
pub fn scan_cli(args: Vec<String>) -> i32 {
    let mut root = None;
    let mut output = None;
    let mut quick = false;
    let mut excludes = Vec::new();
    let mut progress_file: Option<PathBuf> = None;
    let mut cancel_file: Option<PathBuf> = None;
    let mut home: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scan" => { root = args.get(i + 1).cloned(); i += 1; }
            "--output" => { output = args.get(i + 1).cloned(); i += 1; }
            "--quick" => quick = true,
            "--exclude" => { if let Some(v) = args.get(i + 1) { excludes.push(PathBuf::from(v)); } i += 1; }
            "--progress-file" => { progress_file = args.get(i + 1).map(PathBuf::from); i += 1; }
            "--cancel-file" => { cancel_file = args.get(i + 1).map(PathBuf::from); i += 1; }
            "--home" => { home = args.get(i + 1).map(PathBuf::from); i += 1; }
            _ => {}
        }
        i += 1;
    }
    let (Some(root), Some(output)) = (root, output) else {
        eprintln!("usage: disko --scan ROOT --output DB [--quick]");
        return 2;
    };
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(cf) = cancel_file.clone() {
        let c = cancel.clone();
        std::thread::spawn(move || loop {
            if cf.exists() {
                c.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        });
    }
    let opts = scanner::ScanOptions {
        root: PathBuf::from(root),
        output: PathBuf::from(output),
        quick,
        excludes,
        home: home.unwrap_or_else(home_dir),
    };
    use std::io::Write;
    let mut report = |p: scanner::Progress| {
        let line = serde_json::to_string(&p).unwrap_or_default();
        println!("{}", line);
        let _ = std::io::stdout().flush();
        if let Some(pf) = &progress_file {
            let _ = fs::write(pf, &line);
        }
    };
    match scanner::scan(opts, cancel, &mut report) {
        Ok(sum) => {
            println!("{}", serde_json::json!({ "done": true, "summary": sum }));
            0
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "done": true, "error": e }));
            1
        }
    }
}

static SCAN_LIFECYCLE: Mutex<()> = Mutex::const_new(());

#[tauri::command]
async fn run_scan(app: AppHandle, state: State<'_, ProcState>, root: Option<String>, quick: Option<bool>, admin: Option<bool>) -> Result<ScanMeta, String> {
    let _lifecycle = SCAN_LIFECYCLE.try_lock().map_err(|_| "A scan is already running")?;
    if state.scan.lock().await.is_some() {
        return Err("A scan is already running".into());
    }
    let ws = workspace_dir();
    let reports = reports_dir();
    let root = root.unwrap_or_else(|| home_dir().to_string_lossy().to_string());
    let quick = quick.unwrap_or(false);
    let admin = admin.unwrap_or(false);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let out_db = reports.join(format!("{}-{}-{}.sqlite", slug(&root), if quick { "quick" } else { "full" }, stamp));
    let _pending = scan_storage::PendingScan::new(out_db.clone());
    let progress_file = reports.join(format!(".progress-{}.json", stamp));
    let cancel_file = reports.join(format!(".cancel-{}", stamp));
    let _ = fs::remove_file(&cancel_file);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    emit_scan(&app, simple_progress("scan", if admin { "Waiting for administrator approval…" } else { "Starting scan…" }, false, true));

    let mut args: Vec<String> = vec![
        "--scan".into(), root.clone(),
        "--output".into(), out_db.to_string_lossy().to_string(),
        "--exclude".into(), reports.to_string_lossy().to_string(),
        "--progress-file".into(), progress_file.to_string_lossy().to_string(),
        "--cancel-file".into(), cancel_file.to_string_lossy().to_string(),
        "--home".into(), home_dir().to_string_lossy().to_string(),
    ];
    if quick {
        args.push("--quick".into());
    }

    let mut child = if admin {
        // Quote for the shell inside AppleScript.
        let sh_quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let cmd = format!(
            "{} {} ; chown {}:{} {} 2>/dev/null; true",
            sh_quote(&exe.to_string_lossy()),
            args.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "),
            uid,
            gid,
            sh_quote(&out_db.to_string_lossy())
        );
        let script = format!("do shell script \"{}\" with administrator privileges", cmd.replace('\\', "\\\\").replace('"', "\\\""));
        Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start admin scan: {}", e))?
    } else {
        Command::new(&exe)
            .args(&args)
            .current_dir(&ws)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start scanner: {}", e))?
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    *state.scan.lock().await = Some(child.clone());

    // Progress: stream stdout for a normal scan; poll the progress file for an admin scan (osascript buffers output).
    let app2 = app.clone();
    let pf = progress_file.clone();
    let stop_poll = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop_poll.clone();
    let poll_task = if admin {
        Some(tokio::spawn(async move {
            while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(txt) = fs::read_to_string(&pf) {
                    if let Ok(p) = serde_json::from_str::<scanner::Progress>(&txt) {
                        emit_scan(&app2, ScanProgress { phase: p.phase.clone(), message: format!("Scanning as administrator: {}", p.current), entries: p.entries, dirs: p.dirs, bytes: p.bytes, current: p.current, elapsed_ms: p.elapsed_ms, done: false, ok: true });
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }))
    } else {
        None
    };

    let app3 = app.clone();
    let stdout_task = tokio::spawn(async move {
        let mut last: Option<serde_json::Value> = None;
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
                if v.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
                    last = Some(v);
                    continue;
                }
                if let Ok(p) = serde_json::from_value::<scanner::Progress>(v) {
                    emit_scan(&app3, ScanProgress { phase: p.phase.clone(), message: match p.phase.as_str() { "aggregate" => "Adding up folder sizes…".into(), "write" => format!("Writing database{}", if p.current.is_empty() { String::new() } else { format!(": {}", p.current) }), _ => p.current.clone() }, entries: p.entries, dirs: p.dirs, bytes: p.bytes, current: p.current, elapsed_ms: p.elapsed_ms, done: false, ok: true });
                }
            }
        }
        last
    });
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                s.push_str(&line);
                s.push('\n');
            }
        }
        s
    });

    let status = { child.lock().await.wait().await };
    stop_poll.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(t) = poll_task {
        let _ = t.await;
    }
    let last = stdout_task.await.unwrap_or(None);
    let stderr_text = stderr_task.await.unwrap_or_default();
    *state.scan.lock().await = None;
    let _ = fs::remove_file(&progress_file);
    let cancelled = cancel_file.exists();
    let _ = fs::remove_file(&cancel_file);

    let ok = status.as_ref().map(|s| s.success()).unwrap_or(false)
        && scan_info_for(&out_db, &out_db).is_some_and(|s| s.complete);
    if !ok || cancelled {
        let _ = scan_storage::remove_database(&out_db);
        let msg = if cancelled {
            "Scan cancelled".to_string()
        } else if let Some(e) = last.as_ref().and_then(|v| v.get("error")).and_then(|e| e.as_str()) {
            e.to_string()
        } else if stderr_text.contains("User canceled") || stderr_text.contains("-128") {
            "Administrator approval was cancelled".to_string()
        } else if !stderr_text.trim().is_empty() {
            stderr_text.trim().chars().take(400).collect()
        } else {
            "Scan failed".to_string()
        };
        emit_scan(&app, simple_progress("scan", &msg, true, false));
        return Err(msg);
    }

    scan_storage::activate_and_prune(&reports, &out_db)?;

    // Derived reports (best effort, native).
    emit_scan(&app, simple_progress("candidates", "Finding cleanup candidates…", false, true));
    {
        let db = out_db.clone();
        let out = reports.join("candidates.json");
        let home = home_dir();
        let _ = tauri::async_runtime::spawn_blocking(move || reports::write_candidates(&db, &out, &home, 80)).await;
    }
    emit_scan(&app, simple_progress("unused", "Checking for inactive large files…", false, true));
    let _ = reports::write_unused(out_db.clone(), reports.join("unused-files.json"), 90, 100 * 1024 * 1024, 80).await;
    emit_scan(&app, simple_progress("remnants", "Looking for leftovers of uninstalled apps…", false, true));
    {
        let db = out_db.clone();
        let out = remnants::report_path(&reports);
        let _ = tauri::async_runtime::spawn_blocking(move || remnants::write(Some(&db), &out, &home_dir())).await;
    }

    let meta = open_db().and_then(|c| scan_meta(&c)).ok_or("Scan finished but database could not be read")?;
    emit_scan(&app, ScanProgress { phase: "done".into(), message: "Scan complete".into(), entries: meta.entries.unwrap_or(0), dirs: 0, bytes: 0, current: String::new(), elapsed_ms: 0, done: true, ok: true });
    Ok(meta)
}

#[tauri::command]
async fn cancel_scan(state: State<'_, ProcState>) -> Result<bool, String> {
    // Ask politely through the cancel file (works for admin scans too), then kill the child.
    if let Ok(rd) = fs::read_dir(reports_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with(".progress-") {
                let stamp = n.trim_start_matches(".progress-").trim_end_matches(".json");
                let _ = fs::write(reports_dir().join(format!(".cancel-{}", stamp)), "1");
            }
        }
    }
    let child = state.scan.lock().await.clone();
    if let Some(c) = child {
        let _ = c.lock().await.start_kill();
        return Ok(true);
    }
    Ok(false)
}

fn scan_info_for(path: &Path, active: &Path) -> Option<ScanInfo> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut stmt = conn.prepare("SELECT key, value FROM meta").ok()?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()?;
    let mut map: HashMap<String, serde_json::Value> = HashMap::new();
    for (k, v) in rows.flatten() {
        if let Ok(val) = serde_json::from_str(&v) {
            map.insert(k, val);
        }
    }
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Some(ScanInfo {
        path: path.to_string_lossy().to_string(),
        file: path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        root: map.get("root").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        started: map.get("started").and_then(|v| v.as_f64()),
        finished: map.get("finished").and_then(|v| v.as_f64()),
        entries: map.get("entries").and_then(|v| v.as_u64()),
        rows: map.get("rows").and_then(|v| v.as_u64()),
        mode: map.get("mode").and_then(|v| v.as_str()).unwrap_or("thorough").to_string(),
        complete: map.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
        size_bytes: size,
        active: path == active,
    })
}

fn list_scans_sync() -> Result<Vec<ScanInfo>, String> {
    let reports = reports_dir();
    let active = db_path();
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(&reports) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "sqlite").unwrap_or(false) {
                let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if name.starts_with('.') || name.starts_with("home-prev") {
                    continue;
                }
                if let Some(info) = scan_info_for(&p, &active) {
                    out.push(info);
                }
            }
        }
    }
    out.sort_by(|a, b| b.finished.unwrap_or(0.0).partial_cmp(&a.finished.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

#[tauri::command]
async fn list_scans() -> Result<Vec<ScanInfo>, String> {
    tauri::async_runtime::spawn_blocking(list_scans_sync).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_active_scan(path: String) -> Result<(), String> {
    let _lifecycle = SCAN_LIFECYCLE.try_lock().map_err(|_| "A scan is already running")?;
    let p = PathBuf::from(&path);
    if !p.exists() || p.parent() != Some(reports_dir().as_path()) {
        return Err("Scan not found".into());
    }
    scan_storage::activate_and_prune(&reports_dir(), &p)
}

#[tauri::command]
async fn delete_scan(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() || p.parent() != Some(reports_dir().as_path()) {
        return Err("Scan not found".into());
    }
    if p == db_path() {
        return Err("This scan is currently open".into());
    }
    scan_storage::remove_database(&p)
}

// ---------------------------------------------------------------------------
// Commands: Finder, quarantine, restore, purge
// ---------------------------------------------------------------------------

fn reveal_in_finder_sync(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err("File or folder no longer exists".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn open_path_sync(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err("File or folder no longer exists".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_broad_container(path: &str, home: &str) -> bool {
    let containers = [
        "Desktop", "Documents", "Downloads", "Pictures", "Movies", "Music", "Library",
        "Library/Caches", "Library/Logs", "Library/Developer", "Library/Developer/Xcode",
        "Library/Developer/Xcode/DerivedData", "Library/Developer/CoreSimulator",
        "Library/Developer/CoreSimulator/Devices", "Library/Developer/Xcode/iOS DeviceSupport",
        "Library/Developer/Xcode/watchOS DeviceSupport", "Library/Developer/Xcode/tvOS DeviceSupport",
        "Library/Developer/Xcode/visionOS DeviceSupport", "Library/Developer/XcodeBuildMCP",
        "Library/Developer/XcodeBuildMCP/workspaces",
        ".cache", ".npm", ".cargo",
    ];
    containers.iter().any(|c| format!("{}/{}", home, c) == path)
}

/// SF_RESTRICTED: the flag System Integrity Protection puts on paths it seals, including
/// Apple-shipped bundles in /Applications. Renaming one fails with EPERM, so refuse the batch
/// before anything moves rather than aborting halfway.
fn is_sip_restricted(meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_RESTRICTED: u32 = 0x0008_0000;
    meta.st_flags() & SF_RESTRICTED != 0
}


/// Validate canonical paths as well as user-facing paths so aliases cannot bypass protections.
fn validate_cleanup_items(items: &[QuarantineInput], home: &Path) -> Result<(), String> {
    if items.is_empty() { return Err("Nothing selected".into()); }
    let home_str = home.to_string_lossy();
    for it in items {
        let p = Path::new(&it.path);
        if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) {
            return Err("Cleanup requires an absolute path without traversal components".into());
        }
        for candidate in [p.to_path_buf(), fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())] {
            let text = candidate.to_string_lossy();
            let (cat, reason) = classify_path(&text, &home_str);
            if cat == "protected" || is_broad_container(&text, &home_str) || candidate.starts_with(home.join(".Trash")) {
                return Err(format!("Refusing to clean {}: protected path or broad container. {}", text, reason));
            }
        }
        if let Ok(meta) = fs::symlink_metadata(p) {
            if meta.file_type().is_symlink() || fs::canonicalize(p).map_err(|e| e.to_string())? != p {
                return Err(format!("{} uses a symbolic link; select the original location instead", it.path));
            }
            if is_sip_restricted(&meta) { return Err(format!("{} is protected by System Integrity Protection", it.path)); }
            xcode::check_simulator(p, home)?;
            app_cleanup::validate(p, home)?;
        }
    }
    for (i, a) in items.iter().enumerate() {
        if items.iter().skip(i + 1).any(|b| Path::new(&a.path).starts_with(&b.path) || Path::new(&b.path).starts_with(&a.path)) {
            return Err("Overlapping cleanup selections; select each item only once".into());
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct DeleteResult { deleted_items: Vec<String>, failures: Vec<CleanupFailure>, total_bytes: u64 }
#[derive(Debug, Serialize)]
struct CleanupFailure { path: String, error: String }
static CLEANUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn delete_items_sync(items: Vec<QuarantineInput>) -> Result<DeleteResult, String> {
    let _guard = CLEANUP_LOCK.lock().map_err(|_| "Cleanup lock unavailable".to_string())?;
    let home = home_dir();
    let result = delete_items_at(items, &home)?;
    let _ = detach_scan_entries(&result.deleted_items);
    Ok(result)
}

fn delete_items_at(items: Vec<QuarantineInput>, home: &Path) -> Result<DeleteResult, String> {
    validate_cleanup_items(&items, home)?;
    let mut result = DeleteResult { deleted_items: vec![], failures: vec![], total_bytes: 0 };
    for item in items {
        let path = Path::new(&item.path);
        let outcome = (|| -> Result<u64, String> {
            validate_cleanup_items(std::slice::from_ref(&item), &home)?;
            let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            let (size, _) = xcode::allocated_size(path);
            if let Some(id) = xcode::check_simulator(path, &home)? {
                xcode::delete_simulator(&id, &home)?;
            } else if meta.is_dir() {
                fs::remove_dir_all(path).map_err(|e| format!("{}; some files may already have been removed. Rescan before retrying.", e))?;
            } else { fs::remove_file(path).map_err(|e| e.to_string())?; }
            if path.exists() { return Err("The item is still present; refresh and inspect it before trying again".into()); }
            Ok(size)
        })();
        match outcome {
            Ok(size) => { result.total_bytes += size; result.deleted_items.push(item.path); }
            Err(error) => result.failures.push(CleanupFailure { path: item.path, error }),
        }
    }
    Ok(result)
}

#[tauri::command]
async fn delete_items(items: Vec<QuarantineInput>) -> Result<DeleteResult, String> {
    tauri::async_runtime::spawn_blocking(move || delete_items_sync(items)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_app_cleanup_storage(app_id: String) -> Result<app_cleanup::Inventory, String> {
    tauri::async_runtime::spawn_blocking(move || app_cleanup::inventory(&home_dir(), &app_id)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_xcode_storage() -> Result<xcode::Inventory, String> {
    tauri::async_runtime::spawn_blocking(move || xcode::inventory(&home_dir())).await.map_err(|e| e.to_string())
}

fn safe_quarantine_items_sync(items: Vec<QuarantineInput>) -> Result<QuarantineResult, String> {
    let _guard = CLEANUP_LOCK.lock().map_err(|_| "Cleanup lock unavailable".to_string())?;
    let home = home_dir();
    if items.is_empty() {
        return Err("Nothing selected".into());
    }

    validate_cleanup_items(&items, &home)?;

    let now = now_secs();
    let id = format!("disko-{}-{}", now, SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    let trash_dir = home.join(".Trash").join(&id);
    fs::create_dir_all(&trash_dir).map_err(|e| format!("Failed to create quarantine folder: {}", e))?;

    let mut moved = Vec::new();
    let mut skipped = Vec::new();
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;

    for (idx, it) in items.iter().enumerate() {
        let p = Path::new(&it.path);
        if !p.exists() {
            skipped.push(it.path.clone());
            continue;
        }
        let dest = trash_dir.join(format!("{:03}-{}", idx, file_name_of(&it.path)));
        if dest.exists() {
            skipped.push(it.path.clone());
            continue;
        }
        let moved_result = validate_cleanup_items(std::slice::from_ref(it), &home)
            .and_then(|_| fs::rename(p, &dest).map_err(|e| e.to_string()));
        match moved_result {
            Ok(()) => {
                total_bytes += it.size;
                moved.push(it.path.clone());
                entries.push(serde_json::json!({
                    "original": it.path,
                    "quarantined": dest.to_string_lossy(),
                    "size": it.size,
                    "timestamp": now,
                }));
            }
            Err(e) => {
                // Write what we have so far so nothing is lost, then report.
                let journal = serde_json::json!({
                    "id": id, "created_at": now, "total_bytes": total_bytes, "items": entries,
                });
                let _ = fs::write(trash_dir.join("journal.json"), journal.to_string());
                let _ = detach_scan_entries(&moved);
                return Err(format!(
                    "Moved {} item(s) but failed on {}: {}. A recovery journal was written to {}.",
                    moved.len(),
                    it.path,
                    e,
                    trash_dir.display()
                ));
            }
        }
    }

    let journal_path = trash_dir.join("journal.json");
    let journal = serde_json::json!({
        "id": id,
        "created_at": now,
        "total_bytes": total_bytes,
        "items": entries,
    });
    fs::write(&journal_path, serde_json::to_string_pretty(&journal).unwrap_or_default())
        .map_err(|e| format!("Items moved but journal could not be written: {}", e))?;

    // The files are gone; keep the loaded scan honest so views update without a rescan.
    if let Err(e) = detach_scan_entries(&moved) {
        eprintln!("scan update after quarantine failed: {}", e);
    }

    Ok(QuarantineResult {
        journal_path: journal_path.to_string_lossy().to_string(),
        moved_items: moved,
        skipped,
        total_bytes,
    })
}

fn read_journal(j_path: &Path) -> Option<QuarantineJournal> {
    let content = fs::read_to_string(j_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let dir_name = j_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut items = Vec::new();
    if let Some(arr) = val.get("items").and_then(|v| v.as_array()) {
        for it in arr {
            let quarantined = it.get("quarantined").and_then(|v| v.as_str()).unwrap_or("").to_string();
            items.push(QuarantineEntry {
                original: it.get("original").and_then(|v| v.as_str()).unwrap_or("").into(),
                present: !quarantined.is_empty() && Path::new(&quarantined).exists(),
                quarantined,
                size: it.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            });
        }
    }
    let purged = val.get("purged").and_then(|v| v.as_bool()).unwrap_or(false);
    Some(QuarantineJournal {
        id: val.get("id").and_then(|v| v.as_str()).unwrap_or(&dir_name).to_string(),
        path: j_path.to_string_lossy().to_string(),
        created_at: val.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
        total_bytes: val.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
        items,
        purged,
    })
}

fn list_quarantine_journals_sync() -> Result<Vec<QuarantineJournal>, String> {
    let trash = home_dir().join(".Trash");
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(&trash) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("disko-") {
                continue;
            }
            let j_path = entry.path().join("journal.json");
            if let Some(j) = read_journal(&j_path) {
                results.push(j);
            }
        }
    }
    results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(results)
}

fn journal_in_trash(journal_path: &Path) -> Result<PathBuf, String> {
    let trash = home_dir().join(".Trash");
    let canon = fs::canonicalize(journal_path).map_err(|_| "Journal not found".to_string())?;
    let trash_canon = fs::canonicalize(&trash).map_err(|_| "Trash not found".to_string())?;
    if !canon.starts_with(&trash_canon) {
        return Err("Journal is outside ~/.Trash; refusing".into());
    }
    Ok(canon)
}

fn restore_journal_sync(journal_path: String) -> Result<usize, String> {
    let _guard = CLEANUP_LOCK.lock().map_err(|_| "Cleanup lock unavailable".to_string())?;
    let j_file = journal_in_trash(Path::new(&journal_path))?;
    let journal = read_journal(&j_file).ok_or("Invalid journal")?;
    let mut restored = 0usize;
    let mut back = Vec::new();
    for it in &journal.items {
        let quar = Path::new(&it.quarantined);
        let orig = Path::new(&it.original);
        if quar.exists() && !orig.exists() {
            if let Some(parent) = orig.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::rename(quar, orig).is_ok() {
                restored += 1;
                back.push((it.original.clone(), it.size));
            }
        }
    }
    if let Err(e) = reattach_scan_entries(&back) {
        eprintln!("scan update after restore failed: {}", e);
    }
    Ok(restored)
}

fn purge_journal_sync(journal_path: String) -> Result<u64, String> {
    let _guard = CLEANUP_LOCK.lock().map_err(|_| "Cleanup lock unavailable".to_string())?;
    let j_file = journal_in_trash(Path::new(&journal_path))?;
    let journal = read_journal(&j_file).ok_or("Invalid journal")?;
    let trash_canon = fs::canonicalize(home_dir().join(".Trash")).map_err(|e| e.to_string())?;
    let mut freed = 0u64;
    for it in &journal.items {
        let quar = PathBuf::from(&it.quarantined);
        if !quar.exists() {
            continue;
        }
        // Only delete things that really live inside ~/.Trash/disko-*.
        let Ok(canon) = fs::canonicalize(&quar) else { continue };
        if !canon.starts_with(&trash_canon) {
            continue;
        }
        let meta = fs::symlink_metadata(&quar).map_err(|e| e.to_string())?;
        let res = if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(&quar)
        } else {
            fs::remove_file(&quar)
        };
        if res.is_ok() {
            freed += it.size;
        }
    }
    // Mark journal as purged so history keeps the record.
    if let Ok(content) = fs::read_to_string(&j_file) {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&content) {
            val["purged"] = serde_json::Value::Bool(true);
            val["purged_at"] = serde_json::json!(now_secs());
            let _ = fs::write(&j_file, serde_json::to_string_pretty(&val).unwrap_or_default());
        }
    }
    Ok(freed)
}

fn delete_journal_sync(journal_path: String) -> Result<(), String> {
    let j_file = journal_in_trash(Path::new(&journal_path))?;
    let journal = read_journal(&j_file).ok_or("Invalid journal")?;
    if journal.items.iter().any(|i| i.present) {
        return Err("Journal still has quarantined items; restore or purge them first".into());
    }
    if let Some(dir) = j_file.parent() {
        fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn get_coverage_issue_count_sync() -> Result<u64, String> {
    let conn = open_db().ok_or("No scan database loaded")?;
    if let Ok(v) = conn.query_row("SELECT value FROM meta WHERE key='coverage_issues'", [], |r| r.get::<_, String>(0)) {
        if let Ok(n) = v.parse::<u64>() {
            return Ok(n);
        }
    }
    let n: i64 = conn
        .query_row("SELECT count(*) FROM entries WHERE error IS NOT NULL", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(n.max(0) as u64)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LaunchOptions {
    pub start_page: Option<String>,
    pub start_path: Option<String>,
    pub advanced: Option<bool>,
    pub autoscan: Option<String>,
}

/// Re-run the classifier over a scan written by an older rule set and write back only the rows
/// whose verdict actually changed. Scanning a disk takes minutes; a rule change should not need
/// one, and a stale `protected` row would keep an item undraggable for no reason.
fn reclassify_scan_sync() -> Result<usize, String> {
    let conn = match open_db_rw() {
        Some(c) => c,
        None => return Ok(0),
    };
    let stored: u32 = conn
        .query_row("SELECT value FROM meta WHERE key = 'rules'", [], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if stored == classify::RULES_VERSION {
        return Ok(0);
    }
    let home = home_dir().to_string_lossy().to_string();

    // Read pass first: a scan holds millions of rows, and only a handful of verdicts move when a
    // rule changes, so collect the differences instead of rewriting every row.
    let mut changed: Vec<(i64, Classified)> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, path, category, tag, action, cost, reason FROM entries")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let id: i64 = row.get(0).map_err(|e| e.to_string())?;
            let path: String = row.get(1).map_err(|e| e.to_string())?;
            let old_cat: String = row.get::<_, Option<String>>(2).ok().flatten().unwrap_or_default();
            let old_tag: String = row.get::<_, Option<String>>(3).ok().flatten().unwrap_or_default();
            let old_action: String = row.get::<_, Option<String>>(4).ok().flatten().unwrap_or_default();
            let old_cost: String = row.get::<_, Option<String>>(5).ok().flatten().unwrap_or_default();
            let old_reason: String = row.get::<_, Option<String>>(6).ok().flatten().unwrap_or_default();
            let c = classify::classify(&path, &home);
            if c.category == old_cat && c.tag == old_tag && c.action == old_action && c.cost == old_cost && c.reason == old_reason {
                continue;
            }
            changed.push((id, Classified { category: c.category.to_string(), tag: c.tag.to_string(), action: c.action.to_string(), cost: c.cost.to_string(), reason: c.reason }));
        }
    }

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    {
        let mut up = tx
            .prepare("UPDATE entries SET category=?2, tag=?3, action=?4, cost=?5, reason=?6 WHERE id=?1")
            .map_err(|e| e.to_string())?;
        for (id, c) in &changed {
            up.execute(rusqlite::params![id, c.category, c.tag, c.action, c.cost, c.reason])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.execute(
        "INSERT INTO meta(key, value) VALUES('rules', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![classify::RULES_VERSION.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed.len())
}

struct Classified {
    category: String,
    tag: String,
    action: String,
    cost: String,
    reason: String,
}

#[tauri::command]
fn get_launch_options() -> LaunchOptions {
    LaunchOptions {
        start_page: std::env::var("DISKO_START_PAGE").ok(),
        start_path: std::env::var("DISKO_START_PATH").ok(),
        advanced: std::env::var("DISKO_ADVANCED").ok().map(|v| v == "1"),
        autoscan: std::env::var("DISKO_AUTOSCAN").ok(),
    }
}

#[tauri::command]
fn check_full_disk_access() -> bool {
    disk_access::has_full_disk_access(&home_dir())
}

#[tauri::command]
async fn request_full_disk_access() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| {
        if disk_access::request_full_disk_access(&home_dir()) {
            return Ok(true);
        }
        open_full_disk_access_settings()?;
        Ok(false)
    }).await.map_err(|e| e.to_string())?
}

fn open_full_disk_access_settings() -> Result<(), String> {
    let output = std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("Could not open Full Disk Access settings: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

#[tauri::command]
async fn reveal_running_app() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let bundle = disk_access::app_bundle(&executable)
            .ok_or("This is a development executable. Open the built Disko.app to grant access to Disko.")?;
        let output = std::process::Command::new("/usr/bin/open")
            .arg("-R").arg(bundle).output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("Could not show Disko in Finder: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn webview_log(level: String, message: String) {
    use std::io::Write;
    let path = workspace_dir().join(".disko-webview.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} [{}] {}", now_secs(), level, message.chars().take(4000).collect::<String>());
    }
}

#[tauri::command]
fn get_volume() -> VolumeInfo {
    volume_info(&home_dir())
}


// ---------------------------------------------------------------------------
// Async wrappers: keep blocking SQLite / filesystem work off the main thread.
// ---------------------------------------------------------------------------

#[tauri::command]
async fn load_disk_overview() -> Result<OverviewData, String> {
    tauri::async_runtime::spawn_blocking(move || load_disk_overview_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn query_folder_children(path: String, limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    tauri::async_runtime::spawn_blocking(move || query_folder_children_sync(path, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_subtree(path: String, depth: Option<usize>) -> Result<TreeNode, String> {
    tauri::async_runtime::spawn_blocking(move || get_subtree_sync(path, depth))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_path_info(path: String) -> Result<Option<FileItem>, String> {
    tauri::async_runtime::spawn_blocking(move || get_path_info_sync(path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn search_entries(query: String, limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    tauri::async_runtime::spawn_blocking(move || search_entries_sync(query, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_largest_files(limit: Option<usize>) -> Result<Vec<FileItem>, String> {
    tauri::async_runtime::spawn_blocking(move || get_largest_files_sync(limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_candidates() -> Result<Vec<CandidateItem>, String> {
    tauri::async_runtime::spawn_blocking(move || get_candidates_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_unused_files() -> Result<Vec<UnusedItem>, String> {
    tauri::async_runtime::spawn_blocking(move || get_unused_files_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_remnants(force: Option<bool>) -> Result<remnants::RemnantsReport, String> {
    tauri::async_runtime::spawn_blocking(move || get_remnants_sync(force.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_unused_apps() -> Result<Vec<UnusedItem>, String> {
    tauri::async_runtime::spawn_blocking(move || get_unused_apps_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_codex_models() -> Result<CodexModels, String> {
    tauri::async_runtime::spawn_blocking(move || list_codex_models_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn reveal_in_finder(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || reveal_in_finder_sync(path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_path(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open_path_sync(path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn preview_path(path: String) -> Result<(), String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let mut child = Command::new("/usr/bin/qlmanage")
        .arg("-p")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Quick Look stays running while its window is open; reap it without holding up the menu.
    tauri::async_runtime::spawn(async move { let _ = child.wait().await; });
    Ok(())
}

#[tauri::command]
async fn open_in_terminal(path: String) -> Result<(), String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let directory = if path.is_dir() { path.as_path() } else {
        path.parent().ok_or("File has no parent folder")?
    };
    let output = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(directory)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if output.status.success() { Ok(()) } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
async fn safe_quarantine_items(items: Vec<QuarantineInput>) -> Result<QuarantineResult, String> {
    tauri::async_runtime::spawn_blocking(move || safe_quarantine_items_sync(items))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_quarantine_journals() -> Result<Vec<QuarantineJournal>, String> {
    tauri::async_runtime::spawn_blocking(move || list_quarantine_journals_sync())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn restore_journal(journal_path: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || restore_journal_sync(journal_path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn purge_journal(journal_path: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || purge_journal_sync(journal_path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_journal(journal_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || delete_journal_sync(journal_path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_coverage_issue_count() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || get_coverage_issue_count_sync())
        .await
        .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------

pub fn run() {
    tauri::Builder::default()
        .setup(|_| {
            scan_storage::retain_latest(&reports_dir()).map_err(std::io::Error::other)?;
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .manage(ProcState::default())
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            load_disk_overview,
            query_folder_children,
            get_subtree,
            get_path_info,
            search_entries,
            get_largest_files,
            get_candidates,
            get_unused_files,
            get_unused_apps,
            get_remnants,
            list_codex_models,
            codex_chat,
            codex_cancel,
            run_scan,
            cancel_scan,
            list_scans,
            set_active_scan,
            delete_scan,
            reveal_in_finder,
            open_path,
            preview_path,
            open_in_terminal,
            safe_quarantine_items,
            delete_items,
            get_xcode_storage,
            get_app_cleanup_storage,
            list_quarantine_journals,
            restore_journal,
            purge_journal,
            delete_journal,
            get_volume,
            get_coverage_issue_count,
            get_launch_options,
            webview_log,
            check_full_disk_access,
            request_full_disk_access,
            reveal_running_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Disko");
}

#[cfg(test)]
mod reclassify_tests {
    use super::*;

    /// A scan carrying an older rule set is corrected in place: only the rows whose verdict moved
    /// are rewritten, and the stored rule version catches up so the pass runs once.
    #[test]
    fn stale_scan_is_brought_up_to_date() {
        let dir = std::env::temp_dir().join(format!("disko-reclass-{}", now_secs()));
        let reports = dir.join("reports");
        fs::create_dir_all(&reports).unwrap();
        std::env::set_var("DISKO_WORKSPACE", &dir);

        let db = reports.join("home.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE entries(id INTEGER PRIMARY KEY, parent INTEGER, path TEXT UNIQUE,
               kind TEXT, device INTEGER, inode INTEGER, mode INTEGER, size INTEGER,
               mtime_ns INTEGER, ctime_ns INTEGER, allocated INTEGER, total INTEGER,
               logical INTEGER, category TEXT, reason TEXT, error TEXT, tag TEXT, action TEXT, cost TEXT);
             INSERT INTO meta(key, value) VALUES('rules', '1');
             INSERT INTO entries(id, path, kind, category, tag, action, cost, reason)
               VALUES(1, '/Applications/Topaz Photo.app', 'directory', 'protected', 'application', 'owner_app', 'network-large', 'stale'),
                     (2, '/Applications', 'directory', 'protected', 'application', 'owner_app', 'network-large', 'Installed applications; uninstall through the app or its vendor uninstaller.'),
                     (3, '/System/Applications/Mail.app', 'directory', 'protected', 'system-managed', 'report_only', 'unknown', 'macOS system volume; sealed and SIP-protected.');",
        )
        .unwrap();
        drop(conn);

        let changed = reclassify_scan_sync().unwrap();
        assert!(changed >= 1, "the app row should have been corrected");

        let conn = Connection::open(&db).unwrap();
        let cat = |p: &str| -> String {
            conn.query_row("SELECT category FROM entries WHERE path = ?1", rusqlite::params![p], |r| r.get(0)).unwrap()
        };
        assert_eq!(cat("/Applications/Topaz Photo.app"), "review");
        assert_eq!(cat("/Applications"), "protected");
        assert_eq!(cat("/System/Applications/Mail.app"), "protected");
        let rules: String = conn.query_row("SELECT value FROM meta WHERE key='rules'", [], |r| r.get(0)).unwrap();
        assert_eq!(rules, classify::RULES_VERSION.to_string());
        drop(conn);

        // A second pass is a no-op now that the version matches.
        assert_eq!(reclassify_scan_sync().unwrap(), 0);

        std::env::remove_var("DISKO_WORKSPACE");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Apple's own bundles now live in a folder the classifier calls reviewable, so the SIP flag
    /// is what keeps them from being moved. Skipped where the probe path is absent.
    #[test]
    fn sip_flag_is_read_from_a_real_sealed_path() {
        let sealed = Path::new("/System/Applications/Mail.app");
        if let Ok(meta) = fs::symlink_metadata(sealed) {
            assert!(is_sip_restricted(&meta), "SIP-sealed bundle should report restricted");
        }
        let ours = fs::symlink_metadata(std::env::temp_dir()).unwrap();
        assert!(!is_sip_restricted(&ours));
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture() -> PathBuf {
        let p = std::env::temp_dir().join(format!("disko-cleanup-test-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&p).unwrap();
        fs::canonicalize(p).unwrap()
    }
    fn item(p: &Path) -> QuarantineInput { QuarantineInput { path: p.to_string_lossy().into(), size: 99999999 } }
    #[test]
    fn permanent_delete_removes_only_selected_and_reports_missing() {
        let home = fixture();
        let selected = home.join("Downloads/old-build");
        fs::create_dir_all(&selected).unwrap();
        fs::write(selected.join("output"), vec![0; 8192]).unwrap();
        let kept = home.join("Downloads/keep");
        fs::write(&kept, "keep me").unwrap();
        // A link inside the selected tree is unlinked, never followed to its target.
        symlink(&kept, selected.join("alias")).unwrap();
        let missing = home.join("Downloads/gone");
        let result = delete_items_at(vec![item(&selected), item(&missing)], &home).unwrap();
        assert_eq!(result.deleted_items, vec![selected.to_string_lossy().to_string()]);
        assert_eq!(result.failures.len(), 1);
        assert!(!selected.exists());
        assert!(kept.exists());
        assert!(result.total_bytes < 99999999, "do not trust the frontend byte count");
        assert!(!home.join(".Trash").exists(), "permanent deletion bypasses quarantine");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn batch_validation_rejects_aliases_traversal_and_overlap_before_deleting() {
        let home = fixture();
        let target = home.join("Downloads/cache");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("file"), "keep").unwrap();
        let alias = home.join("Downloads/alias");
        symlink(&target, &alias).unwrap();
        for paths in [
            vec![item(&target), item(&home.join("Downloads"))],
            vec![item(&alias)],
            vec![item(&alias.join("file"))],
            vec![item(&home.join("Downloads/../Library"))],
            vec![item(&target), item(&target.join("file"))],
            vec![item(&target), item(&target)],
            vec![item(&home.join(".ssh"))],
            vec![item(&home.join("Library/Developer/CoreSimulator"))],
        ] { assert!(delete_items_at(paths, &home).is_err()); assert!(target.join("file").exists()); }
        fs::remove_dir_all(home).unwrap();
    }
}
