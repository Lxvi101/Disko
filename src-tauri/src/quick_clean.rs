//! Quick Clean: a live, scan-free sweep of the places developer tools and apps refill on their own.
//!
//! Discovery is read-only and bounded: a shallow walk of known project roots plus fixed cache
//! locations. Results are grouped by the program that owns them (Rust, Node.js, Xcode, Docker…)
//! so the UI can show a few clusters instead of hundreds of folders. Cleanup only ever acts on
//! items from the most recent discovery, and re-checks each one right before it is removed.
use crate::{app_cleanup, classify::classify, xcode};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const DAY: u64 = 86_400;
const MIN_ITEM: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickItem {
    pub id: String,
    pub path: String,
    pub label: String,
    pub detail: String,
    pub total: u64,
    pub modified: Option<u64>,
    /// Ticked by default: safe to remove and not in use.
    pub preselect: bool,
    /// Why this item cannot be removed right now.
    pub blocked: Option<String>,
    /// False until the size has been measured; discovery reports items before their size is known.
    #[serde(default)]
    pub measured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickGroup {
    pub id: String,
    /// The program that owns the space, e.g. "Rust" or "Xcode". Groups with the same program are shown together.
    pub program: String,
    pub title: String,
    pub blurb: String,
    /// What happens afterwards: "Rebuilt on the next cargo build."
    pub after: String,
    /// `delete` removes the listed paths; `command` runs the owning tool's own cleanup once.
    pub method: String,
    pub command: Option<String>,
    pub total: u64,
    pub items: Vec<QuickItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuickReport {
    pub generated_at: u64,
    pub groups: Vec<QuickGroup>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuickProgress {
    pub phase: String,
    pub label: String,
    pub index: usize,
    pub count: usize,
    pub found_bytes: u64,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuickFailure { pub label: String, pub error: String }

#[derive(Debug, Clone, Serialize)]
pub struct QuickResult {
    pub freed_bytes: u64,
    pub removed: Vec<String>,
    pub commands: Vec<String>,
    pub failures: Vec<QuickFailure>,
}

static LAST: Mutex<Option<QuickReport>> = Mutex::new(None);

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
fn mtime(path: &Path) -> Option<u64> {
    fs::symlink_metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}
fn name_of(path: &Path) -> String { path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() }
fn is_real_dir(path: &Path) -> bool { fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false) }
fn children(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()).collect()
}
fn short(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) { Ok(rel) => format!("~/{}", rel.display()), Err(_) => path.display().to_string() }
}

/// Newest modification time of a folder and the two levels below it. Build tools touch these
/// (fingerprints, lockfiles, `.package-lock.json`) whenever they run.
fn recent_activity(path: &Path) -> Option<u64> {
    let mut newest = mtime(path);
    for c in children(path) {
        newest = newest.max(mtime(&c));
        if is_real_dir(&c) {
            for g in children(&c).into_iter().take(200) { newest = newest.max(mtime(&g)); }
        }
    }
    newest
}

/// Allocated size of a tree, measured on all cores. Hard links count once; other volumes are skipped.
pub(crate) fn du(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fn walk(path: &Path, dev: u64, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
        let Ok(m) = fs::symlink_metadata(path) else { return 0 };
        if m.dev() != dev { return 0; }
        if !m.is_dir() && m.nlink() > 1 && !seen.lock().map(|mut s| s.insert((m.dev(), m.ino()))).unwrap_or(true) { return 0; }
        let own = m.blocks() * 512;
        if !m.is_dir() { return own; }
        let kids = children(path);
        own + if kids.len() > 4 { kids.par_iter().map(|k| walk(k, dev, seen)).sum::<u64>() } else { kids.iter().map(|k| walk(k, dev, seen)).sum::<u64>() }
    }
    let Ok(m) = fs::symlink_metadata(path) else { return 0 };
    walk(path, m.dev(), &Mutex::new(HashSet::new()))
}

fn age_label(secs: Option<u64>) -> String {
    let Some(t) = secs else { return String::new() };
    let days = now().saturating_sub(t) / DAY;
    match days {
        0 => "used today".into(),
        1 => "used yesterday".into(),
        2..=29 => format!("last used {} days ago", days),
        30..=364 => format!("last used {} mo ago", days / 30),
        _ => format!("last used {} yr ago", days / 365),
    }
}

// ---------------------------------------------------------------------------
// Tools and processes
// ---------------------------------------------------------------------------

fn tool_path(home: &Path, bin: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // GUI apps start with a minimal PATH; look where installers put things too.
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/Applications/OrbStack.app/Contents/MacOS/xbin", "/Applications/Docker.app/Contents/Resources/bin"] { dirs.push(d.into()); }
    for d in [".local/bin", ".cargo/bin", ".bun/bin", "Library/pnpm", ".local/share/pnpm", ".orbstack/bin", ".volta/bin"] { dirs.push(home.join(d)); }
    dirs.into_iter().map(|d| d.join(bin)).find(|p| p.is_file())
}

fn process_list() -> String {
    Command::new("/bin/ps").args(["-axo", "comm=", "-ww"]).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase()).unwrap_or_default()
}
fn running(list: &str, names: &[&str]) -> bool {
    list.lines().any(|l| {
        let base = l.trim().rsplit('/').next().unwrap_or("");
        names.iter().any(|n| base == *n)
    })
}

// ---------------------------------------------------------------------------
// Project folders: build output and dependencies, clustered by program
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind { RustTarget, NodeModules, WebBuild, SwiftBuild, PythonVenv, PythonCache, Pods, Gradle }

struct Found { kind: Kind, path: PathBuf }

fn dev_roots(home: &Path) -> Vec<PathBuf> {
    let names = [
        "Documents/Development", "Development", "Developer", "Projects", "projects", "Code", "code", "src", "dev", "Dev",
        "repos", "Repos", "GitHub", "git", "workspace", "Workspace", "Sites", "Documents/Codex", "Documents/GitHub",
        "Documents/Projects", "Documents/Code", "Desktop", ".codex/worktrees", "conductor", ".worktrees", ".cursor/worktrees",
    ];
    let mut seen = HashSet::new();
    names.iter().map(|n| home.join(n))
        .filter(|p| is_real_dir(p))
        .filter(|p| seen.insert(fs::canonicalize(p).unwrap_or_else(|_| p.clone())))
        .collect()
}

fn classify_dir(path: &Path, name: &str) -> Option<Kind> {
    let parent = path.parent()?;
    let has = |f: &str| parent.join(f).exists();
    match name {
        // A cache marker alone is not enough: the folder must sit next to the manifest that builds it.
        "target" if has("Cargo.toml") && (path.join("CACHEDIR.TAG").exists() || path.join("debug").is_dir() || path.join("release").is_dir()) => Some(Kind::RustTarget),
        "node_modules" if has("package.json") => Some(Kind::NodeModules),
        ".next" | ".nuxt" | ".svelte-kit" | ".turbo" | ".parcel-cache" | ".angular" | ".expo" | ".vercel" if has("package.json") || has("turbo.json") => Some(Kind::WebBuild),
        ".build" if has("Package.swift") => Some(Kind::SwiftBuild),
        ".venv" | "venv" | ".tox" | ".nox" if path.join("pyvenv.cfg").exists() || name == ".tox" || name == ".nox" => Some(Kind::PythonVenv),
        ".pytest_cache" | ".ruff_cache" | ".mypy_cache" | ".uv-cache" => Some(Kind::PythonCache),
        "Pods" if has("Podfile") => Some(Kind::Pods),
        ".gradle" if has("settings.gradle") || has("settings.gradle.kts") || has("build.gradle") || has("build.gradle.kts") => Some(Kind::Gradle),
        _ => None,
    }
}

const NEVER_ENTER: &[&str] = &[".git", "Library", ".Trash", "Applications", "Pictures", "Movies", "Music", "Photos Library.photoslibrary"];

fn walk_projects(dir: &Path, depth: u8, out: &mut Vec<Found>, seen: &mut HashSet<PathBuf>) {
    for f in walk_par(dir, depth) {
        if seen.insert(f.path.clone()) { out.push(f); }
    }
}

/// Parallel, bounded walk. Uses the directory entry's own type so files cost no extra stat.
fn walk_par(dir: &Path, depth: u8) -> Vec<Found> {
    if depth == 0 { return vec![]; }
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let dirs: Vec<PathBuf> = rd.filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path()).collect();
    dirs.into_par_iter().flat_map_iter(|child| {
        let name = name_of(&child);
        if NEVER_ENTER.contains(&name.as_str()) || name.ends_with(".app") || name.ends_with(".photoslibrary") { return vec![]; }
        if let Some(kind) = classify_dir(&child, &name) { return vec![Found { kind, path: child }]; } // never descend into generated trees
        // Other generated trees we don't offer but must not walk through.
        if matches!(name.as_str(), "DerivedData" | "__pycache__" | "dist" | "build" | ".cache" | "vendor" | "Pods" | ".gradle" | ".idea" | ".vscode") && depth < 6 { return vec![]; }
        walk_par(&child, depth - 1)
    }).collect()
}

/// "my-app", or "my-app · Codex worktree" for agent checkouts.
fn project_label(project: &Path, home: &Path) -> (String, String) {
    let mut name = name_of(project);
    // "service" or "src-tauri" alone says little; name the repository too.
    const GENERIC: &[&str] = &["src-tauri", "service", "services", "app", "apps", "web", "www", "frontend", "backend", "server", "client", "api", "packages", "macos", "ios", "desktop", "mobile", "core", "src", "rust", "native", "platform", "site", "docs", "example", "examples", "demo"];
    if GENERIC.contains(&name.to_lowercase().as_str()) {
        if let Some(parent) = project.parent().map(name_of).filter(|p| !p.is_empty() && !p.starts_with("agent-")) {
            name = format!("{} › {}", parent, name);
        }
    }
    let s = project.to_string_lossy();
    let origin = if s.contains("/.codex/worktrees/") { " · Codex worktree" }
        else if s.contains("/.claude/worktrees/") { " · Claude worktree" }
        else if s.contains("/.cursor/worktrees/") { " · Cursor worktree" }
        else if s.contains("/.worktrees/") || s.contains("/conductor/") { " · worktree" }
        else { "" };
    (format!("{}{}", name, origin), short(project, home))
}

fn project_groups(home: &Path, procs: &str) -> Vec<QuickGroup> {
    let mut found = vec![];
    let mut seen = HashSet::new();
    for root in dev_roots(home) {
        let depth = if root.ends_with("Desktop") { 3 } else { 7 };
        walk_projects(&root, depth, &mut found, &mut seen);
    }
    // Project folders inside other agents' checkouts under each project: `.claude/worktrees`.
    let cargo_busy = running(procs, &["cargo", "rustc", "rust-analyzer-proc-macro-srv"]);
    let swift_busy = running(procs, &["swift-build", "swift-frontend", "swiftpm-xctest-helper"]);

    let sized: Vec<(Kind, QuickItem)> = found.into_par_iter().filter_map(|f| {
        let project = f.path.parent()?.to_path_buf();
        let (label, detail) = project_label(&project, home);
        let activity = recent_activity(&f.path);
        let idle_days = activity.map(|t| now().saturating_sub(t) / DAY).unwrap_or(9999);
        let hot = activity.is_some_and(|t| now().saturating_sub(t) < 3600);
        let busy = match f.kind { Kind::RustTarget => cargo_busy && hot, Kind::SwiftBuild => swift_busy && hot, _ => hot };
        let blocked = busy.then(|| "Something is building here right now.".to_string());
        // Dependencies need a reinstall, so only tick ones that haven't been touched for a while.
        let threshold = match f.kind { Kind::PythonCache | Kind::WebBuild => 3, Kind::RustTarget | Kind::SwiftBuild | Kind::Gradle => 7, _ => 14 };
        let item = QuickItem {
            id: format!("p:{}", f.path.display()), path: f.path.to_string_lossy().into(), label,
            detail: format!("{} · {}", detail, age_label(activity)), total: 0, modified: activity,
            preselect: blocked.is_none() && idle_days >= threshold, blocked, measured: false,
        };
        Some((f.kind, item))
    }).collect();

    let spec: &[(Kind, &str, &str, &str, &str, &str)] = &[
        (Kind::RustTarget, "rust-target", "Rust", "Build folders", "Compiled crates, incremental caches and binaries in each project's target folder.", "Rebuilt on the next cargo build. The first build takes longer."),
        (Kind::NodeModules, "node-modules", "Node.js", "node_modules", "Installed packages for each JavaScript project.", "Run npm, pnpm, yarn or bun install to get them back."),
        (Kind::WebBuild, "web-build", "Node.js", "Framework build caches", ".next, .turbo, .nuxt and similar output.", "Regenerated by the next dev server or build."),
        (Kind::SwiftBuild, "swift-build", "Swift", "Package builds", "SwiftPM .build folders with compiled products and indexes.", "Rebuilt on the next swift build."),
        (Kind::PythonVenv, "python-venv", "Python", "Virtual environments", "Python environments inside projects (.venv, venv, .tox).", "Recreate with uv sync, poetry install or pip install -r."),
        (Kind::PythonCache, "python-cache", "Python", "Test and lint caches", "pytest, ruff and mypy caches.", "Refilled the next time the tool runs."),
        (Kind::Pods, "pods", "Xcode", "CocoaPods", "Pods folders inside iOS projects.", "Run pod install to get them back."),
        (Kind::Gradle, "gradle-project", "Java & Android", "Gradle project caches", "Per-project .gradle folders.", "Rebuilt on the next Gradle sync."),
    ];
    spec.iter().filter_map(|(kind, id, program, title, blurb, after)| {
        let mut items: Vec<QuickItem> = sized.iter().filter(|(k, _)| k == kind).map(|(_, i)| i.clone()).collect();
        if items.is_empty() { return None; }
        items.sort_by(|a, b| b.total.cmp(&a.total));
        Some(group(id, program, title, blurb, after, items))
    }).collect()
}

fn group(id: &str, program: &str, title: &str, blurb: &str, after: &str, items: Vec<QuickItem>) -> QuickGroup {
    QuickGroup {
        id: id.into(), program: program.into(), title: title.into(), blurb: blurb.into(), after: after.into(),
        method: "delete".into(), command: None, total: items.iter().map(|i| i.total).sum(), items,
    }
}

// ---------------------------------------------------------------------------
// Fixed cache locations
// ---------------------------------------------------------------------------

fn dir_item(path: PathBuf, label: &str, home: &Path, preselect: bool) -> Option<QuickItem> {
    if !is_real_dir(&path) && !fs::symlink_metadata(&path).map(|m| m.is_file()).unwrap_or(false) { return None; }
    let modified = mtime(&path);
    Some(QuickItem { id: format!("p:{}", path.display()), detail: short(&path, home), path: path.to_string_lossy().into(), label: label.into(), total: 0, modified, preselect, blocked: None, measured: false })
}

struct CacheSpec {
    id: &'static str, program: &'static str, title: &'static str, blurb: &'static str, after: &'static str,
    paths: &'static [&'static str], preselect: bool,
    /// The tool's own cleanup, used instead of deleting when the tool is installed.
    command: Option<(&'static str, &'static [&'static str])>,
    /// Processes that must not be running while this is removed.
    busy: &'static [&'static str],
}

static CACHES: &[CacheSpec] = &[
    CacheSpec { id: "npm", program: "Node.js", title: "npm download cache", blurb: "Every package version npm has ever downloaded.", after: "Packages download again the next time you install them.", paths: &[".npm/_cacache"], preselect: true, command: None, busy: &["npm"] },
    CacheSpec { id: "pnpm", program: "Node.js", title: "pnpm store", blurb: "pnpm's shared package store.", after: "Only packages no project uses any more are removed.", paths: &["Library/pnpm/store", ".local/share/pnpm/store", ".pnpm-store"], preselect: true, command: Some(("pnpm", &["store", "prune"])), busy: &["pnpm"] },
    CacheSpec { id: "yarn", program: "Node.js", title: "Yarn cache", blurb: "Yarn's package cache.", after: "Packages download again when needed.", paths: &["Library/Caches/Yarn", ".yarn/berry/cache"], preselect: true, command: None, busy: &["yarn"] },
    CacheSpec { id: "bun", program: "Node.js", title: "Bun cache", blurb: "Bun's global install cache.", after: "Packages download again when needed.", paths: &[".bun/install/cache"], preselect: true, command: None, busy: &["bun"] },
    CacheSpec { id: "node-tools", program: "Node.js", title: "Build tool downloads", blurb: "Electron, node-gyp and TypeScript download caches.", after: "Downloaded again on the next build.", paths: &["Library/Caches/electron", "Library/Caches/electron-builder", "Library/Caches/node-gyp", "Library/Caches/typescript", ".node-gyp"], preselect: true, command: None, busy: &[] },
    CacheSpec { id: "playwright", program: "Node.js", title: "Test browsers", blurb: "Browsers downloaded by Playwright, Puppeteer and Cypress.", after: "Large downloads: each browser is several hundred MB.", paths: &["Library/Caches/ms-playwright", ".cache/puppeteer", "Library/Caches/Cypress"], preselect: false, command: None, busy: &[] },
    CacheSpec { id: "uv", program: "Python", title: "uv cache", blurb: "Wheels, sources and builds cached by uv.", after: "uv keeps what your environments still use.", paths: &[".cache/uv"], preselect: true, command: Some(("uv", &["cache", "prune"])), busy: &["uv"] },
    CacheSpec { id: "pip", program: "Python", title: "pip cache", blurb: "Downloaded wheels and HTTP responses.", after: "Packages download again when needed.", paths: &["Library/Caches/pip", ".cache/pip"], preselect: true, command: None, busy: &["pip", "pip3"] },
    CacheSpec { id: "poetry", program: "Python", title: "Poetry cache", blurb: "Poetry's package and artifact cache.", after: "Packages download again when needed.", paths: &["Library/Caches/pypoetry"], preselect: true, command: None, busy: &["poetry"] },
    CacheSpec { id: "huggingface", program: "AI models", title: "Downloaded models", blurb: "Hugging Face model downloads.", after: "Models can be many GB and take a while to download again. Fine-tunes you trained yourself are never stored here.", paths: &[".cache/huggingface/hub"], preselect: false, command: None, busy: &[] },
    CacheSpec { id: "cargo-registry", program: "Rust", title: "Crate downloads", blurb: "Downloaded crates and their unpacked sources.", after: "Cargo downloads them again on the next build. Installed tools stay.", paths: &[".cargo/registry/cache", ".cargo/registry/src", ".cargo/git/checkouts"], preselect: false, command: None, busy: &["cargo", "rustc"] },
    CacheSpec { id: "go", program: "Go", title: "Go build cache", blurb: "Compiled Go packages.", after: "Rebuilt on the next go build.", paths: &["Library/Caches/go-build"], preselect: true, command: None, busy: &["go"] },
    CacheSpec { id: "gradle", program: "Java & Android", title: "Gradle caches", blurb: "Downloaded dependencies and build caches.", after: "Downloaded again on the next Gradle sync.", paths: &[".gradle/caches"], preselect: false, command: None, busy: &["java"] },
    CacheSpec { id: "brew", program: "Homebrew", title: "Old downloads", blurb: "Bottles and source archives Homebrew kept after installing.", after: "Homebrew only removes what it no longer needs.", paths: &["Library/Caches/Homebrew"], preselect: true, command: Some(("brew", &["cleanup", "--prune=all", "-s"])), busy: &["brew"] },
    CacheSpec { id: "cocoapods", program: "Xcode", title: "CocoaPods cache", blurb: "Downloaded pod specs and sources.", after: "Downloaded again on the next pod install.", paths: &["Library/Caches/CocoaPods"], preselect: true, command: None, busy: &["pod"] },
    CacheSpec { id: "swiftpm", program: "Swift", title: "Package cache", blurb: "Repositories SwiftPM cloned for package resolution.", after: "Cloned again on the next package resolve.", paths: &["Library/Caches/org.swift.swiftpm"], preselect: true, command: None, busy: &["swift-build", "xcodebuild"] },
];

fn cache_groups(home: &Path, procs: &str) -> Vec<QuickGroup> {
    CACHES.par_iter().filter_map(|spec| {
        let items: Vec<QuickItem> = spec.paths.iter()
            .filter_map(|rel| dir_item(home.join(rel), rel.rsplit('/').next().unwrap_or(rel), home, spec.preselect))
            .collect::<Vec<_>>().into_iter()
            .map(|mut i| {
                i.label = spec.title.to_string();
                if !spec.busy.is_empty() && running(procs, spec.busy) {
                    i.blocked = Some(format!("{} is running. Try again when it's done.", spec.busy[0]));
                    i.preselect = false;
                }
                i
            })
            .collect();
        if items.is_empty() { return None; }
        let several = items.len() > 1;
        let items: Vec<QuickItem> = items.into_iter().map(|mut i| { if several { i.label = i.detail.clone(); } i }).collect();
        let mut g = group(spec.id, spec.program, spec.title, spec.blurb, spec.after, items);
        if let Some((bin, args)) = spec.command {
            if let Some(path) = tool_path(home, bin) {
                g.method = "command".into();
                g.command = Some(format!("{} {}", path.display(), args.join(" ")));
                g.after = format!("{} (runs `{} {}`)", spec.after, bin, args.join(" "));
            } else if spec.id == "uv" || spec.id == "pnpm" {
                return None; // without the tool we can't tell which entries are still in use
            }
        }
        Some(g)
    }).collect()
}

// ---------------------------------------------------------------------------
// Xcode
// ---------------------------------------------------------------------------

fn xcode_groups(home: &Path, procs: &str) -> Vec<QuickGroup> {
    let building = running(procs, &["xcodebuild", "xcbbuildservice", "swift-frontend"]);
    let dev = home.join("Library/Developer");
    let mut out = vec![];

    let mut derived: Vec<QuickItem> = children(&dev.join("Xcode/DerivedData")).into_iter().filter(|p| is_real_dir(p)).filter_map(|p| {
        let raw = name_of(&p);
        let name = match raw.rsplit_once('-') { Some((n, hash)) if hash.len() >= 20 => n.to_string(), _ => raw.clone() };
        let workspace = plist::Value::from_file(p.join("info.plist")).ok()
            .and_then(|v| v.as_dictionary().and_then(|d| d.get("WorkspacePath")).and_then(|w| w.as_string()).map(|w| w.replace(&*home.to_string_lossy(), "~")));
        let mut it = dir_item(p.clone(), &name, home, true)?;
        let activity = recent_activity(&p);
        if building && activity.is_some_and(|t| now().saturating_sub(t) < 3600) { it.blocked = Some("Xcode is building this project right now.".into()); it.preselect = false; }
        it.detail = format!("{} · {}", workspace.unwrap_or(it.detail), age_label(activity));
        Some(it)
    }).collect();
    derived.sort_by(|a, b| a.label.cmp(&b.label));
    if !derived.is_empty() { out.push(group("derived-data", "Xcode", "DerivedData", "Build products, indexes and logs for every project Xcode has opened.", "Rebuilt the next time you build. Indexing runs again.", derived)); }

    let caches: Vec<QuickItem> = [("Library/Caches/com.apple.dt.Xcode", "Xcode cache"), ("Library/Caches/com.apple.dt.xcodebuild", "xcodebuild cache"), ("Library/Developer/CoreSimulator/Caches", "Simulator caches")]
        .iter().filter_map(|(rel, label)| dir_item(home.join(rel), label, home, !building)).collect();
    if !caches.is_empty() { out.push(group("xcode-caches", "Xcode", "Xcode & simulator caches", "Caches kept by Xcode, xcodebuild and the simulator.", "Refilled as you work. Quit Xcode and Simulator first.", caches)); }

    // Simulators whose runtime is gone can never boot again. Only these are measured.
    if let Ok(json) = xcode::simulator_devices(home) {
        let dead: Vec<QuickItem> = json["devices"].as_object().into_iter().flat_map(|o| o.iter())
            .flat_map(|(runtime, list)| list.as_array().into_iter().flatten().map(move |d| (runtime.clone(), d.clone())))
            .filter(|(_, d)| d["isAvailable"].as_bool() == Some(false))
            .filter_map(|(runtime, d)| {
                let udid = d["udid"].as_str()?;
                let mut it = dir_item(dev.join("CoreSimulator/Devices").join(udid), d["name"].as_str().unwrap_or(udid), home, true)?;
                it.detail = format!("{} · runtime no longer installed", runtime.rsplit('.').next().unwrap_or("").replace('-', " "));
                if d["state"].as_str() != Some("Shutdown") { it.blocked = Some("Shut down this simulator first.".into()); it.preselect = false; }
                Some(it)
            }).collect();
        if !dead.is_empty() { out.push(group("dead-simulators", "Xcode", "Broken simulators", "Simulators for iOS versions you no longer have installed.", "Removed through simctl, the same way Xcode deletes them.", dead)); }
    }

    // Debug symbols for OS versions older than the newest one stored for the same device model.
    let mut support = vec![];
    for platform in ["iOS", "watchOS", "tvOS", "visionOS"] {
        let root = dev.join(format!("Xcode/{} DeviceSupport", platform));
        let entries: Vec<(PathBuf, String, Vec<u32>)> = children(&root).into_iter().filter(|p| is_real_dir(p)).map(|p| { let (m, v) = xcode::support_version(&name_of(&p)); (p, m, v) }).collect();
        for (p, model, ver) in &entries {
            if entries.iter().any(|(_, m, v)| m == model && v > ver) {
                if let Some(mut it) = dir_item(p.clone(), &name_of(p), home, false) { it.detail = format!("{} debug symbols · a newer version is stored", platform); support.push(it); }
            }
        }
    }
    if !support.is_empty() { out.push(group("device-support", "Xcode", "Old device support", "Debug symbols for iOS versions your devices no longer run.", "Xcode copies them again the next time you connect a device on that version.", support)); }
    out
}

// ---------------------------------------------------------------------------
// Docker
// ---------------------------------------------------------------------------

fn parse_size(s: &str) -> u64 {
    let s = s.split_whitespace().next().unwrap_or("").trim();
    let idx = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: f64 = num.parse().unwrap_or(0.0);
    let mult = match unit.to_ascii_lowercase().as_str() { "kb" | "k" => 1e3, "mb" | "m" => 1e6, "gb" | "g" => 1e9, "tb" | "t" => 1e12, _ => 1.0 };
    (n * mult) as u64
}

fn docker_groups(home: &Path, procs: &str) -> (Vec<QuickGroup>, Option<String>) {
    let Some(docker) = tool_path(home, "docker") else { return (vec![], None) };
    // Measuring would start a stopped Docker VM. Only look when it is already running.
    let up = procs.lines().any(|l| l.contains("orbstack") || l.contains("com.docker.") || l.contains("dockerd") || l.contains("colima") || l.contains("vmnetd"));
    if !up { return (vec![], Some("Docker isn't running, so its build cache and images weren't measured. Start it and scan again to include them.".into())); }
    let out = Command::new(&docker).args(["system", "df", "--format", "{{json .}}"]).output();
    let Ok(out) = out else { return (vec![], None) };
    if !out.status.success() { return (vec![], None); }
    let mut groups = vec![];
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let kind = v["Type"].as_str().unwrap_or("");
        let reclaim = parse_size(v["Reclaimable"].as_str().unwrap_or("0"));
        if reclaim < MIN_ITEM { continue; }
        let (id, title, blurb, after, args, pre): (&str, &str, &str, &str, &str, bool) = match kind {
            "Build Cache" => ("docker-build", "Build cache", "Layers BuildKit kept from earlier image builds.", "The next docker build starts from scratch for changed steps.", "builder prune -af", true),
            "Images" => ("docker-images", "Unused images", "Images no container uses, including old base images.", "Pulled again when needed. Images you built and never pushed are gone for good, so review first.", "image prune -af", false),
            _ => continue, // containers and volumes are never touched
        };
        let item = QuickItem { id: format!("cmd:{}", id), path: String::new(), label: title.into(), detail: format!("docker {}", args), total: reclaim, modified: None, preselect: pre, blocked: None, measured: true };
        let mut g = group(id, "Docker", title, blurb, after, vec![item]);
        g.method = "command".into();
        g.command = Some(format!("{} {}", docker.display(), args));
        groups.push(g);
    }
    (groups, None)
}

// ---------------------------------------------------------------------------
// System: Trash, logs, app caches, installers, old agent versions
// ---------------------------------------------------------------------------

fn system_groups(home: &Path, procs: &str, notes: &mut Vec<String>) -> Vec<QuickGroup> {
    let mut out = vec![];

    // Trash
    match fs::read_dir(home.join(".Trash")) {
        Ok(rd) => {
            let items: Vec<QuickItem> = rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| name_of(p) != ".DS_Store")
                .collect::<Vec<_>>().into_par_iter().filter_map(|p| { let n = name_of(&p); dir_item(p, &n, home, true) }).collect();
            if !items.is_empty() {
                let mut items = items; items.sort_by(|a, b| b.total.cmp(&a.total));
                out.push(group("trash", "macOS", "Trash", "Things you already threw away.", "Deleted for good. This can't be undone.", items));
            }
        }
        Err(_) => notes.push("Disko can't see your Trash. Give it Full Disk Access to include it.".into()),
    }

    // Logs older than a week
    let week_ago = now().saturating_sub(7 * DAY);
    let mut logs: Vec<PathBuf> = vec![];
    for root in [home.join("Library/Logs"), home.join("Library/Logs/DiagnosticReports")] {
        for p in children(&root) {
            if name_of(&p) == "DiagnosticReports" || name_of(&p).starts_with('.') { continue; }
            if recent_activity(&p).is_some_and(|t| t < week_ago) { logs.push(p); }
        }
    }
    let mut log_items: Vec<QuickItem> = logs.into_par_iter().filter_map(|p| { let n = name_of(&p); dir_item(p, &n, home, true) }).collect();
    if !log_items.is_empty() {
        log_items.sort_by(|a, b| b.total.cmp(&a.total));
        out.push(group("logs", "macOS", "Old logs & crash reports", "App logs and crash reports nothing has written to for a week.", "Only useful while troubleshooting.", log_items));
    }

    // App caches in ~/Library/Caches, judged by the same rules as the rest of Disko.
    let handled: Vec<String> = CACHES.iter().flat_map(|c| c.paths.iter()).map(|p| home.join(p).to_string_lossy().to_string()).collect();
    let home_s = home.to_string_lossy().to_string();
    let candidates: Vec<PathBuf> = children(&home.join("Library/Caches")).into_iter()
        .filter(|p| !handled.iter().any(|h| h.starts_with(&p.to_string_lossy().to_string())))
        .filter(|p| { let c = classify(&p.to_string_lossy(), &home_s); c.category == "rebuildable" && c.action == "quarantine" })
        .filter(|p| !name_of(p).starts_with("com.apple.") || name_of(p) == "com.apple.dt.Xcode")
        .filter(|p| name_of(p) != "com.apple.dt.Xcode" && name_of(p) != "com.apple.dt.xcodebuild")
        .collect();
    let mut cache_items: Vec<QuickItem> = candidates.into_par_iter().filter_map(|p| {
        let raw = name_of(&p);
        let pretty = pretty_bundle(&raw);
        let mut item = dir_item(p, &pretty, home, true)?;
        let needle = pretty.to_lowercase();
        if needle.len() > 2 && procs.lines().any(|l| l.contains(&format!("/{}.app/", needle)) || l.rsplit('/').next() == Some(needle.as_str())) {
            item.preselect = false;
            item.detail = format!("{} · app is open", item.detail);
        }
        Some(item)
    }).collect();
    if !cache_items.is_empty() {
        cache_items.sort_by(|a, b| b.total.cmp(&a.total));
        out.push(group("app-caches", "Apps", "App caches", "Files apps download or generate to load faster.", "Apps rebuild them as you use them. Caches of open apps aren't ticked.", cache_items));
    }

    // Disk images and installer packages in Downloads that have sat around for a month.
    let month_ago = now().saturating_sub(30 * DAY);
    let mut installers: Vec<QuickItem> = children(&home.join("Downloads")).into_iter().filter(|p| {
        let n = name_of(p).to_lowercase();
        (n.ends_with(".dmg") || n.ends_with(".pkg") || n.ends_with(".xip") || n.ends_with(".mpkg")) && mtime(p).is_some_and(|t| t < month_ago)
    }).filter_map(|p| { let n = name_of(&p); let mut it = dir_item(p, &n, home, true)?; it.detail = format!("{} · {}", it.detail, age_label(it.modified).replace("last used", "downloaded")); Some(it) }).collect();
    if !installers.is_empty() {
        installers.sort_by(|a, b| b.total.cmp(&a.total));
        out.push(group("installers", "macOS", "Old installers", "Disk images and packages in Downloads from more than a month ago.", "Download them again if you ever need to reinstall.", installers));
    }

    // Old Claude Code / Cursor agent versions (current and newest are protected by app_cleanup).
    for (id, program) in [("claude", "Claude Code"), ("cursor", "Cursor")] {
        let Ok(inv) = app_cleanup::inventory(home, id) else { continue };
        let items: Vec<QuickItem> = inv.items.iter().filter(|i| i.group == "version" && i.blocked.is_none() && i.total >= MIN_ITEM).map(|i| QuickItem {
            id: format!("p:{}", i.path), path: i.path.clone(), label: format!("Version {}", i.name), detail: short(Path::new(&i.path), home),
            total: i.total, modified: i.modified, preselect: true, blocked: None, measured: true,
        }).collect();
        if !items.is_empty() {
            out.push(group(&format!("{}-versions", id), "AI agents", &format!("Old {} versions", program), "Earlier copies left behind by auto-updates.", "The version you use and the newest one are always kept.", items));
        }
    }
    out
}

fn pretty_bundle(raw: &str) -> String {
    let known = [("slackmacgap", "Slack"), ("spotify", "Spotify"), ("hnc", "Discord"), ("microsoft", "Microsoft"), ("todesktop", "Desktop app")];
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() >= 3 && ["com", "net", "org", "io", "app", "dev", "co", "us", "me", "ai"].contains(&parts[0]) {
        for (k, v) in known { if parts[1..].iter().any(|p| p.eq_ignore_ascii_case(k)) { return v.into(); } }
        let generic = ["client", "app", "desktop", "mac", "macos", "helper", "main", "electron"];
        let pick = if generic.contains(&parts[2].to_lowercase().as_str()) { parts[1].to_string() } else { parts[2..].join(".") };
        let pick = pick.replace(['-', '_'], " ");
        let mut c = pick.chars();
        return c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(pick);
    }
    raw.to_string()
}

// ---------------------------------------------------------------------------
// Discovery entry point
// ---------------------------------------------------------------------------

/// What the UI hears while discovery runs: groups as soon as they're found, then sizes as they're measured.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuickEvent {
    Found { groups: Vec<QuickGroup> },
    Sized { sizes: Vec<(String, u64)> },
    Step { label: String, done: usize, count: usize },
}

enum Msg { Found(Vec<QuickGroup>, Vec<String>), Sized(String, u64) }

static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs every source at once. Each one reports its groups as soon as it has located them, then
/// its folders are measured in parallel; sizes stream out in small batches.
pub fn discover(home: &Path, emit: &mut dyn FnMut(QuickEvent)) -> Result<QuickReport, String> {
    use std::sync::atomic::Ordering;
    if SCANNING.swap(true, Ordering::SeqCst) { return Err("Quick Clean is already scanning".into()); }
    struct Reset;
    impl Drop for Reset { fn drop(&mut self) { SCANNING.store(false, Ordering::SeqCst); } }
    let _reset = Reset;

    let procs = process_list();
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let home_buf = home.to_path_buf();
    let worker = std::thread::spawn(move || {
        let home = home_buf.as_path();
        let procs = procs.as_str();
        rayon::scope(|s| {
            for source in 0..5 {
                let tx = tx.clone();
                s.spawn(move |_| {
                    let mut notes = vec![];
                    let groups = match source {
                        0 => project_groups(home, procs),
                        1 => cache_groups(home, procs),
                        2 => xcode_groups(home, procs),
                        3 => { let (g, n) = docker_groups(home, procs); notes.extend(n); g }
                        _ => system_groups(home, procs, &mut notes),
                    };
                    let pending: Vec<(String, String)> = groups.iter().flat_map(|g| g.items.iter()).filter(|i| !i.measured).map(|i| (i.id.clone(), i.path.clone())).collect();
                    let _ = tx.send(Msg::Found(groups, notes));
                    pending.into_par_iter().for_each(|(id, path)| { let _ = tx.send(Msg::Sized(id, du(Path::new(&path)))); });
                });
            }
        });
    });

    let mut groups: Vec<QuickGroup> = vec![];
    let mut notes = vec![];
    let mut sources = 0;
    let (mut todo, mut done) = (0usize, 0usize);
    let mut batch: Vec<(String, u64)> = vec![];
    let mut last = std::time::Instant::now();
    let step = |sources: usize, todo: usize, done: usize| QuickEvent::Step {
        label: if sources < 5 { "Looking around".into() } else { format!("Measuring {} of {} folders", done, todo) },
        done: sources + done, count: 5 + todo.max(done),
    };
    emit(step(0, 0, 0));
    for msg in rx {
        match msg {
            Msg::Found(g, n) => {
                sources += 1;
                todo += g.iter().flat_map(|g| &g.items).filter(|i| !i.measured).count();
                notes.extend(n);
                emit(QuickEvent::Found { groups: g.clone() });
                groups.extend(g);
                emit(step(sources, todo, done));
            }
            Msg::Sized(id, size) => {
                done += 1;
                if let Some(item) = groups.iter_mut().flat_map(|g| g.items.iter_mut()).find(|i| i.id == id) { item.total = size; item.measured = true; }
                batch.push((id, size));
                if last.elapsed().as_millis() > 120 {
                    emit(QuickEvent::Sized { sizes: std::mem::take(&mut batch) });
                    emit(step(sources, todo, done));
                    last = std::time::Instant::now();
                }
            }
        }
    }
    let _ = worker.join();
    if !batch.is_empty() { emit(QuickEvent::Sized { sizes: batch }); }

    for g in &mut groups {
        g.items.retain(|i| i.total >= MIN_ITEM);
        g.items.sort_by(|a, b| b.total.cmp(&a.total));
    }
    // Drop any item nested in another one (a project inside another project's node_modules, etc.).
    let all: Vec<String> = groups.iter().flat_map(|g| g.items.iter().map(|i| i.path.clone())).filter(|p| !p.is_empty()).collect();
    for g in &mut groups {
        g.items.retain(|i| i.path.is_empty() || !all.iter().any(|o| o != &i.path && i.path.starts_with(&format!("{}/", o))));
        g.total = g.items.iter().map(|i| i.total).sum();
    }
    groups.retain(|g| !g.items.is_empty() && g.total >= 4 * MIN_ITEM);
    groups.sort_by(|a, b| b.total.cmp(&a.total));
    let report = QuickReport { generated_at: now(), groups, notes };
    if let Ok(mut last) = LAST.lock() { *last = Some(report.clone()); }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Re-validate a path from the last discovery just before deleting it.
fn check_path(path: &Path, home: &Path, group_id: &str) -> Result<(), String> {
    if !path.is_absolute() || !path.starts_with(home) || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("Outside your home folder".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|_| "Already gone".to_string())?;
    if meta.file_type().is_symlink() { return Err("Is a symbolic link".into()); }
    if fs::canonicalize(path).map_err(|e| e.to_string())? != path { return Err("Path resolves elsewhere".into()); }
    let in_trash = path.parent() == Some(&home.join(".Trash"));
    let c = classify(&path.to_string_lossy(), &home.to_string_lossy());
    if c.category == "protected" && !in_trash { return Err(format!("Protected: {}", c.reason)); }
    if group_id != "trash" && in_trash { return Err("Unexpected Trash item".into()); }
    // Something may have started building here since discovery.
    if group_id.starts_with("rust") || group_id.ends_with("build") || group_id == "derived-data" || group_id == "node-modules" || group_id == "python-venv" {
        if recent_activity(path).is_some_and(|t| now().saturating_sub(t) < 120) { return Err("Started changing a moment ago; skipped".into()); }
    }
    xcode::check_simulator(path, home)?;
    app_cleanup::validate(path, home)?;
    Ok(())
}

fn remove(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        // Builds can leave read-only files (Go module cache, some Rust outputs). Make them writable first.
        let r = fs::remove_dir_all(path);
        if r.is_err() && path.exists() {
            let _ = Command::new("/bin/chmod").args(["-R", "u+w"]).arg(path).output();
            return fs::remove_dir_all(path);
        }
        r
    } else {
        fs::remove_file(path)
    }
}

pub fn clean(home: &Path, ids: &[String], progress: &mut dyn FnMut(QuickProgress)) -> Result<QuickResult, String> {
    let report = LAST.lock().map_err(|_| "Busy")?.clone().ok_or("Run Quick Clean's scan first")?;
    let wanted: HashSet<&String> = ids.iter().collect();
    let mut work: Vec<(&QuickGroup, Vec<&QuickItem>)> = report.groups.iter()
        .map(|g| (g, g.items.iter().filter(|i| wanted.contains(&i.id) && i.blocked.is_none()).collect::<Vec<_>>()))
        .filter(|(_, items)| !items.is_empty()).collect();
    work.sort_by_key(|(g, _)| if g.method == "command" { 1 } else { 0 });
    let count: usize = work.iter().map(|(g, items)| if g.method == "command" { 1 } else { items.len() }).sum();
    let mut result = QuickResult { freed_bytes: 0, removed: vec![], commands: vec![], failures: vec![] };
    let mut index = 0;
    for (g, items) in work {
        if g.method == "command" {
            let label = g.title.clone();
            progress(QuickProgress { phase: "clean".into(), label: format!("{}: {}", g.program, label), index, count, found_bytes: 0, freed_bytes: result.freed_bytes });
            let cmd = g.command.clone().unwrap_or_default();
            let mut parts = cmd.split(' ');
            let bin = parts.next().unwrap_or("");
            let mut c = Command::new(bin);
            c.args(parts);
            // Homebrew and friends need a PATH that GUI apps don't get.
            let path = format!("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:{}", std::env::var("PATH").unwrap_or_default());
            c.env("PATH", path).env("HOMEBREW_NO_AUTO_UPDATE", "1").env("HOMEBREW_NO_ENV_HINTS", "1").stdin(std::process::Stdio::null());
            let before: u64 = items.iter().map(|i| i.total).sum();
            let measure = || items.iter().filter(|i| !i.path.is_empty()).map(|i| xcode::allocated_size(Path::new(&i.path)).0).sum::<u64>();
            match c.output() {
                Ok(o) if o.status.success() => {
                    let freed = if items.iter().all(|i| i.path.is_empty()) { before } else { before.saturating_sub(measure()) };
                    result.freed_bytes += freed;
                    result.commands.push(cmd.clone());
                }
                Ok(o) => result.failures.push(QuickFailure { label, error: String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect() }),
                Err(e) => result.failures.push(QuickFailure { label, error: e.to_string() }),
            }
            index += 1;
            continue;
        }
        for item in items {
            progress(QuickProgress { phase: "clean".into(), label: item.label.clone(), index, count, found_bytes: 0, freed_bytes: result.freed_bytes });
            index += 1;
            let path = Path::new(&item.path);
            let outcome = check_path(path, home, &g.id).and_then(|_| {
                if let Some(id) = xcode::simulator_id(path, home)? { xcode::delete_simulator(&id, home) } else { remove(path).map_err(|e| e.to_string()) }
            });
            match outcome {
                Ok(()) => { result.freed_bytes += item.total; result.removed.push(item.path.clone()); }
                Err(e) if e == "Already gone" => {}
                Err(e) => result.failures.push(QuickFailure { label: format!("{} ({})", item.label, g.title), error: e }),
            }
        }
    }
    progress(QuickProgress { phase: "done".into(), label: "Done".into(), index: count, count, found_bytes: 0, freed_bytes: result.freed_bytes });
    // Items that were removed shouldn't be offered again.
    if let Ok(mut last) = LAST.lock() {
        if let Some(r) = last.as_mut() {
            let gone: HashSet<&String> = result.removed.iter().collect();
            for g in &mut r.groups { g.items.retain(|i| !gone.contains(&i.path)); }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("disko-quick-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(d).unwrap()
    }

    #[test]
    fn only_folders_next_to_their_manifest_count() {
        let root = tmp("manifest");
        let app = root.join("app");
        fs::create_dir_all(app.join("node_modules/x")).unwrap();
        fs::create_dir_all(app.join("target/debug")).unwrap();
        fs::write(app.join("package.json"), "{}").unwrap();
        // A random folder called target without Cargo.toml is somebody's data.
        let other = root.join("photos/target");
        fs::create_dir_all(other.join("debug")).unwrap();
        let mut out = vec![];
        walk_projects(&root, 5, &mut out, &mut HashSet::new());
        let kinds: Vec<_> = out.iter().map(|f| (f.kind, f.path.clone())).collect();
        assert!(kinds.iter().any(|(k, p)| *k == Kind::NodeModules && p == &app.join("node_modules")));
        assert!(!kinds.iter().any(|(k, _)| *k == Kind::RustTarget));
        fs::write(app.join("Cargo.toml"), "").unwrap();
        let mut out = vec![];
        walk_projects(&root, 5, &mut out, &mut HashSet::new());
        assert!(out.iter().any(|f| f.kind == Kind::RustTarget && f.path == app.join("target")));
        assert!(!out.iter().any(|f| f.path.starts_with(app.join("node_modules/x"))));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discover_then_clean_removes_only_what_was_picked() {
        let home = tmp("e2e");
        let app = home.join("Development/old-app");
        let modules = app.join("node_modules/left-pad");
        fs::create_dir_all(&modules).unwrap();
        fs::write(app.join("package.json"), "{}").unwrap();
        fs::write(app.join("index.js"), "code").unwrap();
        fs::write(modules.join("big.bin"), vec![1u8; 2 * 1024 * 1024]).unwrap();
        // Untouched for a year, so it counts as idle.
        Command::new("/usr/bin/touch").args(["-t", "202501010000"]).arg(app.join("node_modules")).arg(&modules).arg(modules.join("big.bin")).status().unwrap();
        let report = discover(&home, &mut |_| {}).unwrap();
        let g = report.groups.iter().find(|g| g.id == "node-modules").expect("node_modules found");
        let item = &g.items[0];
        assert_eq!(item.path, app.join("node_modules").to_string_lossy());
        assert!(item.preselect);
        let r = clean(&home, &[item.id.clone()], &mut |_| {}).unwrap();
        assert!(r.failures.is_empty(), "{:?}", r.failures);
        assert!(!app.join("node_modules").exists());
        assert!(app.join("index.js").exists() && app.join("package.json").exists());
        // Unknown ids are ignored rather than trusted.
        let r = clean(&home, &["p:/etc".into()], &mut |_| {}).unwrap();
        assert!(r.removed.is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn docker_sizes_parse() {
        assert_eq!(parse_size("17.12GB (100%)"), 17_120_000_000);
        assert_eq!(parse_size("6.254GB"), 6_254_000_000);
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("512kB"), 512_000);
    }

    #[test]
    fn cleanup_refuses_paths_outside_home_and_links() {
        let home = tmp("check");
        let outside = std::env::temp_dir();
        assert!(check_path(&outside, &home, "x").is_err());
        let real = home.join("real"); fs::create_dir_all(&real).unwrap();
        let link = home.join("link"); std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(check_path(&link, &home, "x").is_err());
        assert!(check_path(&home.join("missing"), &home, "x").is_err());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bundle_names_read_well() {
        assert_eq!(pretty_bundle("com.spotify.client"), "Spotify");
        assert_eq!(pretty_bundle("com.tinyspeck.slackmacgap"), "Slack");
        assert_eq!(pretty_bundle("com.figma.Desktop"), "Figma");
        assert_eq!(pretty_bundle("Google"), "Google");
    }
}
