//! Disko app shell: Tauri commands and events on top of the workspace crates (see AGENTS.md).
//! Also the headless entry points: `--mcp` (MCP server for agents) and `--bench` (scan + findings
//! summary as JSON, used to compare releases).

mod backend;
mod hub;

use disko_core::{CleanFailure, CleanResult, Finding, Trashed, TreeNode};
use disko_memory::{Memory, MemoryOp};
use hub::Hub;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

type HubState = Arc<Hub>;

/// Run blocking work off the main thread.
async fn bg<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct BootInfo {
    version: String,
    home: String,
    full_disk_access: bool,
    volume: disko_core::VolumeInfo,
    snapshot_age_s: Option<u64>,
}

#[tauri::command]
async fn boot(app: AppHandle, hub: State<'_, HubState>) -> Result<BootInfo, String> {
    let hub = hub.inner().clone();
    let version = app.package_info().version.to_string();
    bg(move || {
        hub.start();
        BootInfo {
            version,
            home: disko_core::paths::home().to_string_lossy().into(),
            full_disk_access: disko_macos::has_full_disk_access(),
            volume: hub.volume(),
            snapshot_age_s: hub.snapshot_age().map(|d| d.as_secs()),
        }
    })
    .await
}

#[tauri::command]
fn rescan(hub: State<'_, HubState>) {
    hub.rescan();
}

#[tauri::command]
async fn tree(hub: State<'_, HubState>, path: Option<String>, depth: Option<u32>) -> Result<Option<TreeNode>, String> {
    let hub = hub.inner().clone();
    bg(move || hub.engine.tree(path.as_deref(), depth.unwrap_or(2).min(8))).await
}

#[tauri::command]
async fn search(hub: State<'_, HubState>, query: String, limit: Option<usize>) -> Result<Vec<TreeNode>, String> {
    let hub = hub.inner().clone();
    bg(move || hub.engine.search(&query, limit.unwrap_or(50))).await
}

#[tauri::command]
fn clutter(hub: State<'_, HubState>) -> Vec<Finding> {
    hub.visible_findings()
}

#[tauri::command]
async fn clean(hub: State<'_, HubState>, ids: Vec<String>, paths: Vec<String>) -> Result<CleanResult, String> {
    let hub = hub.inner().clone();
    bg(move || hub.clean(&ids, &[], &paths)).await
}

#[derive(Serialize)]
struct UndoResult {
    restored: usize,
    failed: Vec<CleanFailure>,
}

#[tauri::command]
async fn undo(hub: State<'_, HubState>, trashed: Vec<Trashed>) -> Result<UndoResult, String> {
    let hub = hub.inner().clone();
    bg(move || {
        let (restored, failed) = hub.undo(&trashed);
        UndoResult { restored, failed }
    })
    .await
}

#[tauri::command]
async fn apps() -> Result<Value, String> {
    bg(|| json!(disko_apps::apps())).await
}

/// One app's storage page (`AppInfo.storage` says which apps have one). Slow-ish (simctl, sizes):
/// runs off the main thread.
#[tauri::command]
async fn app_storage(hub: State<'_, HubState>, path: String) -> Result<Option<disko_apps::AppStorage>, String> {
    let hub = hub.inner().clone();
    bg(move || hub.app_storage(&path)).await
}

#[tauri::command]
async fn uninstall(hub: State<'_, HubState>, paths: Vec<String>) -> Result<CleanResult, String> {
    let hub = hub.inner().clone();
    bg(move || hub.uninstall(&paths)).await
}

#[tauri::command]
async fn sessions() -> Result<Value, String> {
    bg(|| json!(disko_apps::sessions())).await
}

#[tauri::command]
fn memory(hub: State<'_, HubState>) -> Memory {
    hub.memory()
}

#[tauri::command]
async fn memory_update(hub: State<'_, HubState>, op: MemoryOp) -> Result<Memory, String> {
    let hub = hub.inner().clone();
    bg(move || hub.memory_update(op)).await?
}

#[tauri::command]
async fn routine_run(hub: State<'_, HubState>, id: String) -> Result<CleanResult, String> {
    let hub = hub.inner().clone();
    bg(move || hub.run_routine(&id)).await?
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    disko_macos::reveal(&path)
}

#[tauri::command]
fn open(path: String) -> Result<(), String> {
    disko_macos::open(&path)
}

#[tauri::command]
fn quicklook(path: String) -> Result<(), String> {
    disko_macos::quicklook(&path)
}

#[tauri::command]
fn open_full_disk_access() -> Result<(), String> {
    disko_macos::open_full_disk_access()
}

#[tauri::command]
fn full_disk_access() -> bool {
    disko_macos::has_full_disk_access()
}

#[tauri::command]
async fn icon(key: String, size: Option<u32>) -> Result<Option<String>, String> {
    let path = key.strip_prefix("app:").unwrap_or(&key).to_string();
    if !path.starts_with('/') {
        return Ok(None);
    }
    bg(move || disko_macos::icon_data_url(&path, size.unwrap_or(32).clamp(16, 256))).await
}

#[derive(Serialize)]
struct McpClient {
    id: String,
    name: String,
    present: bool,
    connected: bool,
}

fn mcp_clients() -> Vec<McpClient> {
    disko_mcp::install_targets()
        .into_iter()
        .map(|t| McpClient {
            id: serde_json::to_value(t.target).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            name: t.name,
            present: t.detected,
            connected: t.installed,
        })
        .collect()
}

#[tauri::command]
async fn mcp_status() -> Result<Vec<McpClient>, String> {
    bg(mcp_clients).await
}

#[tauri::command]
async fn mcp_install(client: String) -> Result<Vec<McpClient>, String> {
    bg(move || {
        let target: disko_mcp::Target = serde_json::from_value(json!(client)).map_err(|_| format!("Unknown agent: {client}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        disko_mcp::install(target, &exe)?;
        Ok(mcp_clients())
    })
    .await?
}

/// Panics (on any thread) go to ~/Library/Logs/Disko/disko.log, so a misbehaving long-running app
/// leaves evidence. The default hook still runs.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = disko_core::paths::home().join("Library/Logs/Disko");
        if std::fs::create_dir_all(&dir).is_ok() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("disko.log")) {
                let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let thread = std::thread::current().name().unwrap_or("?").to_string();
                let _ = writeln!(f, "[{when}] panic on {thread}: {info}\n{}", std::backtrace::Backtrace::force_capture());
            }
        }
        default(info);
    }));
}

pub fn run() {
    log_panics();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let emit: hub::Emit = Arc::new(move |name: &str, payload: Value| {
                let _ = handle.emit(name, payload);
            });
            app.manage::<HubState>(hub::create(emit, false));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(hub) = window.app_handle().try_state::<HubState>() {
                    let _ = hub.engine.save_snapshot();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            boot,
            rescan,
            tree,
            search,
            clutter,
            clean,
            undo,
            apps,
            app_storage,
            uninstall,
            sessions,
            memory,
            memory_update,
            routine_run,
            reveal,
            open,
            quicklook,
            open_full_disk_access,
            full_disk_access,
            icon,
            mcp_status,
            mcp_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Disko");
}

/// `Disko --mcp`: MCP server on stdio. Uses the app's snapshot and FSEvents to be current within
/// seconds; scans only when there's no usable snapshot.
pub fn mcp_cli() -> i32 {
    log_panics();
    let emit: hub::Emit = Arc::new(|_, _| {});
    let hub = hub::create(emit, true);
    hub.start();
    let server = disko_mcp::Server::with_default_store(Arc::new(backend::HubBackend(hub.clone())));
    let code = server.serve_stdio();
    let _ = hub.engine.save_snapshot();
    code
}

/// `Disko --json apps|sessions`: print that inventory as JSON (for debugging and scripts).
pub fn json_cli(args: &[String]) -> i32 {
    let what = args.iter().skip_while(|a| *a != "--json").nth(1).map(String::as_str).unwrap_or("");
    let v = match what {
        "apps" => json!(disko_apps::apps()),
        "sessions" => json!(disko_apps::sessions()),
        _ => {
            eprintln!("usage: Disko --json apps|sessions");
            return 2;
        }
    };
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    0
}

/// `Disko --bench [--out FILE]`: one full scan plus all probes, then a JSON summary on stdout
/// (time, files, clutter by category). Read-only.
pub fn bench_cli(args: &[String]) -> i32 {
    let started = std::time::Instant::now();
    let emit: hub::Emit = Arc::new(|_, _| {});
    let hub = hub::create(emit, true);
    let first = std::sync::Mutex::new(None::<u128>);
    hub.rescan();
    // Wait for the scan thread to start and finish.
    std::thread::sleep(std::time::Duration::from_millis(300));
    while hub.is_scanning() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut f = first.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if f.is_none() && !hub.all_findings().is_empty() {
            *f = Some(started.elapsed().as_millis());
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let findings = hub.visible_findings();
    let mut by_cat: std::collections::BTreeMap<String, (u64, usize, u64)> = Default::default();
    for f in &findings {
        let k = serde_json::to_value(f.category).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        let e = by_cat.entry(k).or_default();
        e.0 += f.bytes;
        e.1 += 1;
        if f.safety == disko_core::Safety::Safe {
            e.2 += f.bytes;
        }
    }
    let total: u64 = findings.iter().map(|f| f.bytes).sum();
    let safe: u64 = findings.iter().filter(|f| f.safety == disko_core::Safety::Safe).map(|f| f.bytes).sum();
    let p = hub.progress();
    let snapshot = disko_core::paths::cache_dir().join("snapshot.bin");
    let out = json!({
        "elapsed_ms": started.elapsed().as_millis(),
        "first_finding_ms": *first.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        "files": p.files,
        "dirs": p.dirs,
        "scanned_bytes": p.bytes,
        "findings": findings.len(),
        "clutter_bytes": total,
        "clutter_gb": (total as f64 / 1e9 * 100.0).round() / 100.0,
        "safe_gb": (safe as f64 / 1e9 * 100.0).round() / 100.0,
        "by_category": by_cat.iter().map(|(k, (b, n, s))| json!({"category": k, "gb": (*b as f64 / 1e9 * 100.0).round() / 100.0, "items": n, "safe_gb": (*s as f64 / 1e9 * 100.0).round() / 100.0})).collect::<Vec<_>>(),
        "top": findings.iter().take(40).map(|f| json!({"title": f.title, "detail": f.detail, "category": f.category, "gb": (f.bytes as f64 / 1e9 * 100.0).round() / 100.0, "safety": f.safety, "why": f.why})).collect::<Vec<_>>(),
        "volume": hub.volume(),
        "snapshot_bytes": std::fs::metadata(&snapshot).map(|m| m.len()).unwrap_or(0),
        "rss_bytes": rss(),
    });
    let text = serde_json::to_string_pretty(&out).unwrap_or_default();
    if let Some(i) = args.iter().position(|a| a == "--out") {
        if let Some(f) = args.get(i + 1) {
            let _ = std::fs::write(PathBuf::from(f), &text);
        }
    }
    println!("{text}");
    0
}

fn rss() -> u64 {
    // Peak resident set size of this process.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage fills the struct we own.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
        usage.ru_maxrss as u64
    } else {
        0
    }
}
