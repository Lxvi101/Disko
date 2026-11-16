//! Uninstalled-app leftovers ("remnants"): an identity graph rather than a list of folder names.
//!
//! 1. Inventory installed bundle identifiers from the app folders (primary), Spotlight (secondary)
//!    and running processes (live owners).
//! 2. Enumerate the Library roots where apps leave state, keyed by exact bundle identifier.
//! 3. Score each absent identity with the weights from the research brief; shared vendor state and
//!    privacy-sensitive apps pull the score down, Apple and password managers are vetoed outright.
//! 4. Emit one logical group per app so the UI can offer "612 MB across cache, state and 14 small
//!    files" instead of dozens of tiny suggestions.

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemnantMember {
    pub path: String,
    /// cache | http | saved-state | logs | webkit | state | container | group | prefs | scripts | cookies
    pub kind: String,
    pub total: u64,
    pub newest_mtime: Option<i64>,
    /// True when the member is cache-like and the group is high confidence.
    pub preselect: bool,
    /// True for persistent state that may hold documents or settings.
    pub persistent: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemnantGroup {
    pub id: String,
    pub name: String,
    pub score: i32,
    /// likely | confirm
    pub confidence: String,
    pub total: u64,
    pub newest_mtime: Option<i64>,
    pub members: Vec<RemnantMember>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OrphanAgent {
    pub path: String,
    pub label: String,
    pub program: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemnantsReport {
    pub schema: u32,
    pub rules: u32,
    pub generated_at: u64,
    pub db: String,
    pub installed_apps: usize,
    pub groups: Vec<RemnantGroup>,
    pub agents: Vec<OrphanAgent>,
}

const APP_DIRS: &[&str] = &["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities", "/Library/CoreServices"];

/// Vendors whose prefix says nothing about shared ownership (hosting namespaces and defaults).
const GENERIC_VENDORS: &[&str] = &["com.github", "io.github", "com.gitlab", "com.electron", "com.example", "com.yourcompany", "org.example"];

/// Never proposed: the id belongs to Apple, a credential store, or a security product.
const VETO: &[&str] = &["com.apple.", "com.1password", "com.agilebits", "com.bitwarden", "org.keepassx", "com.keepassxc", "com.lastpass", "com.dashlane", "in.sinew.enpass", "com.nordpass", "com.keepersecurity", "com.strongbox", "com.objective-see", "com.malwarebytes"];

/// Profile, mail, chat: state that persists past uninstall on purpose.
const SENSITIVE: &[&str] = &["org.mozilla", "com.google.chrome", "com.microsoft.edgemac", "com.brave.browser", "company.thebrowser", "com.vivaldi", "com.operasoftware", "com.tinyspeck", "com.hnc.discord", "ru.keepcoder.telegram", "net.whatsapp", "com.microsoft.teams", "org.whispersystems.signal", "com.readdle.smartemail", "com.microsoft.outlook", "com.freron.mailmate"];

fn bundle_id_of(app: &Path) -> Option<(String, String)> {
    let info = app.join("Contents/Info.plist");
    let v = plist::Value::from_file(&info).ok()?;
    let d = v.as_dictionary()?;
    let id = d.get("CFBundleIdentifier")?.as_string()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let name = d
        .get("CFBundleDisplayName")
        .or_else(|| d.get("CFBundleName"))
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| app.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    Some((id, name))
}

fn scan_app_dir(dir: &Path, depth: u8, out: &mut HashMap<String, String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(".app") {
            if let Some((id, n)) = bundle_id_of(&p) {
                out.entry(id.to_ascii_lowercase()).or_insert(n);
            }
        } else if depth > 0 && p.is_dir() && !name.starts_with('.') {
            scan_app_dir(&p, depth - 1, out);
        }
    }
}

fn run_with_timeout(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(cmd).args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Bundle ids Spotlight knows about anywhere (other volumes, Downloads, nested helpers).
fn spotlight_ids() -> HashSet<String> {
    let mut set = HashSet::new();
    let Some(out) = run_with_timeout("/usr/bin/mdfind", &["-attr", "kMDItemCFBundleIdentifier", "kMDItemContentType == 'com.apple.application-bundle'"], Duration::from_secs(12)) else { return set };
    for line in out.lines() {
        if let Some(idx) = line.find("kMDItemCFBundleIdentifier = ") {
            let v = line[idx + "kMDItemCFBundleIdentifier = ".len()..].trim().trim_matches('"');
            if !v.is_empty() && v != "(null)" {
                set.insert(v.to_ascii_lowercase());
            }
        }
    }
    set
}

/// Bundle ids of apps with a running process.
fn running_ids() -> HashSet<String> {
    let mut set = HashSet::new();
    let Some(out) = run_with_timeout("/bin/ps", &["-axo", "comm="], Duration::from_secs(5)) else { return set };
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    for line in out.lines() {
        let l = line.trim();
        let Some(idx) = l.find(".app/Contents/MacOS/") else { continue };
        let app = l[..idx + 4].to_string();
        let id = seen.entry(app.clone()).or_insert_with(|| bundle_id_of(Path::new(&app)).map(|(id, _)| id.to_ascii_lowercase())).clone();
        if let Some(id) = id {
            set.insert(id);
        }
    }
    set
}

/// Installer receipts: historical evidence that an id was once installed as a package.
fn receipt_ids() -> HashSet<String> {
    run_with_timeout("/usr/sbin/pkgutil", &["--pkgs"], Duration::from_secs(5))
        .map(|o| o.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// True when `s` looks like a reverse-DNS bundle identifier (three or more components).
fn is_bundle_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        && parts[0].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Strip a leading team identifier ("ABCDE12345.") or "group." from a Group Containers name.
fn group_identity(name: &str) -> Option<String> {
    let s = name.strip_prefix("group.").unwrap_or(name);
    let s = match s.split_once('.') {
        Some((team, rest)) if team.len() == 10 && team.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) => rest,
        _ => s,
    };
    is_bundle_id(s).then(|| s.to_string())
}

fn vendor_key(id: &str) -> String {
    let parts: Vec<&str> = id.split('.').collect();
    let two = parts.iter().take(2).cloned().collect::<Vec<_>>().join(".");
    if GENERIC_VENDORS.contains(&two.as_str()) {
        parts.iter().take(3).cloned().collect::<Vec<_>>().join(".")
    } else {
        two
    }
}

struct Root {
    dir: &'static str,
    kind: &'static str,
    suffix: &'static str,
    persistent: bool,
}

const ROOTS: &[Root] = &[
    Root { dir: "Library/Caches", kind: "cache", suffix: "", persistent: false },
    Root { dir: "Library/HTTPStorages", kind: "http", suffix: "", persistent: false },
    Root { dir: "Library/Saved Application State", kind: "saved-state", suffix: ".savedState", persistent: false },
    Root { dir: "Library/Logs", kind: "logs", suffix: "", persistent: false },
    Root { dir: "Library/WebKit", kind: "webkit", suffix: "", persistent: true },
    Root { dir: "Library/Application Support", kind: "state", suffix: "", persistent: true },
    Root { dir: "Library/Containers", kind: "container", suffix: "", persistent: true },
    Root { dir: "Library/Group Containers", kind: "group", suffix: "", persistent: true },
    Root { dir: "Library/Preferences", kind: "prefs", suffix: ".plist", persistent: true },
    Root { dir: "Library/Application Scripts", kind: "scripts", suffix: "", persistent: true },
    Root { dir: "Library/Cookies", kind: "cookies", suffix: ".binarycookies", persistent: true },
];

struct Stats {
    total: u64,
    newest: Option<i64>,
}

/// Size and newest modification from the scan; falls back to the filesystem for folded files.
fn stats_for(conn: Option<&Connection>, path: &str) -> Stats {
    if let Some(conn) = conn {
        let lo = format!("{}/", path);
        let hi = format!("{}0", path);
        // Two index-friendly queries; an OR across both forms made SQLite scan the table.
        let own: Option<(Option<i64>, Option<i64>)> = conn.query_row("SELECT total, mtime_ns FROM entries WHERE path = ?1", [path], |r| Ok((r.get(0)?, r.get(1)?))).ok();
        if let Some((total, own_mtime)) = own {
            let below: Option<i64> = conn.query_row("SELECT max(mtime_ns) FROM entries WHERE path >= ?1 AND path < ?2", rusqlite::params![lo, hi], |r| r.get(0)).ok().flatten();
            let newest = [own_mtime, below].into_iter().flatten().max();
            return Stats { total: total.unwrap_or(0).max(0) as u64, newest: newest.map(|n| n / 1_000_000_000) };
        }
    }
    let meta = fs::symlink_metadata(path).ok();
    let newest = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    let total = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0);
    Stats { total, newest }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn orphan_agents(home: &Path) -> Vec<OrphanAgent> {
    let mut out = Vec::new();
    let dir = home.join("Library/LaunchAgents");
    let Ok(rd) = fs::read_dir(&dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "plist").unwrap_or(true) {
            continue;
        }
        let Ok(v) = plist::Value::from_file(&p) else { continue };
        let Some(d) = v.as_dictionary() else { continue };
        let label = d.get("Label").and_then(|v| v.as_string()).unwrap_or("").to_string();
        if label.starts_with("com.apple.") {
            continue;
        }
        let program = d
            .get("Program")
            .and_then(|v| v.as_string())
            .map(|s| s.to_string())
            .or_else(|| d.get("ProgramArguments").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_string()).map(|s| s.to_string()))
            .unwrap_or_default();
        if !program.starts_with('/') {
            continue; // bare command names resolve through PATH; not worth a false positive
        }
        if !Path::new(&program).exists() {
            out.push(OrphanAgent {
                path: p.to_string_lossy().to_string(),
                label,
                program: program.clone(),
                reason: format!("Launch agent points at {} which no longer exists; launchd will keep failing to start it.", program),
            });
        }
    }
    out
}

pub fn build(db: Option<&Path>, home: &Path) -> RemnantsReport {
    let conn = db.and_then(|d| Connection::open_with_flags(d, OpenFlags::SQLITE_OPEN_READ_ONLY).ok());

    // 1. inventory
    let mut installed: HashMap<String, String> = HashMap::new();
    for d in APP_DIRS {
        scan_app_dir(Path::new(d), 1, &mut installed);
    }
    scan_app_dir(&home.join("Applications"), 1, &mut installed);
    let spotlight = spotlight_ids();
    let running = running_ids();
    let receipts = receipt_ids();
    let installed_vendors: HashSet<String> = installed.keys().map(|id| vendor_key(id)).collect();
    let owned = |id: &str| -> bool {
        installed.contains_key(id) || installed.keys().any(|k| id.starts_with(&format!("{}.", k)) || k.starts_with(&format!("{}.", id)))
    };
    let live = |id: &str| -> bool { running.iter().any(|r| r == id || id.starts_with(&format!("{}.", r)) || r.starts_with(&format!("{}.", id))) };

    // 2. leftovers keyed by identity
    let mut by_id: HashMap<String, Vec<(String, &'static str, bool)>> = HashMap::new();
    for root in ROOTS {
        let dir = home.join(root.dir);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !root.suffix.is_empty() && !name.ends_with(root.suffix) {
                continue;
            }
            let base = name.trim_end_matches(root.suffix);
            let identity = if root.kind == "group" { group_identity(base) } else { is_bundle_id(base).then(|| base.to_string()) };
            let Some(identity) = identity else { continue };
            let key = identity.to_ascii_lowercase();
            if VETO.iter().any(|v| key.starts_with(v)) {
                continue;
            }
            by_id.entry(key).or_default().push((e.path().to_string_lossy().to_string(), root.kind, root.persistent));
        }
    }

    // 3. score
    let now_s = now() as i64;
    let mut groups = Vec::new();
    for (id, members) in by_id {
        if owned(&id) || spotlight.contains(&id) || live(&id) {
            continue;
        }
        let mut evidence = vec![format!("No installed app has the bundle id {}", id)];
        let mut score = 4 + 2 + 1; // absent from app folders, from Spotlight, and from running processes
        evidence.push("Spotlight finds no app bundle with this id".into());
        evidence.push("No running process belongs to it".into());

        let mut out_members = Vec::new();
        let mut total = 0u64;
        let mut newest: Option<i64> = None;
        for (path, kind, persistent) in &members {
            let st = stats_for(conn.as_ref(), path);
            total += st.total;
            if let Some(n) = st.newest {
                newest = Some(newest.map_or(n, |x: i64| x.max(n)));
            }
            out_members.push(RemnantMember { path: path.clone(), kind: kind.to_string(), total: st.total, newest_mtime: st.newest, preselect: false, persistent: *persistent });
        }
        let stale = newest.map(|n| now_s - n >= 90 * 86_400).unwrap_or(false);
        if stale {
            score += 1;
            evidence.push("Nothing inside has changed for 90 days or more".into());
        }
        if out_members.iter().any(|m| m.kind == "cache" || m.kind == "http") {
            score += 2;
            evidence.push("Includes a cache folder".into());
        }
        // Only real apps leave preferences, containers, saved state or receipts. A bare cache
        // folder under a reverse-DNS name may belong to a library or tool, not an uninstalled app.
        let app_evidence = out_members.iter().any(|m| matches!(m.kind.as_str(), "prefs" | "saved-state" | "container" | "state" | "webkit" | "group" | "scripts"));
        let had_receipt = receipts.iter().any(|r| r == &id || r.starts_with(&format!("{}.", id)));
        if had_receipt {
            score += 1;
            evidence.push("An installer receipt shows it was installed as a package".into());
        }
        if !app_evidence && !had_receipt {
            score -= 3;
            evidence.push("No preferences, container or saved state found; this may be a library or tool cache rather than an app".into());
            if total < 50 * 1_048_576 {
                continue;
            }
        }
        let vendor = vendor_key(&id);
        if installed_vendors.contains(&vendor) {
            score -= 3;
            evidence.push(format!("Other apps from {} are installed and may share this data", vendor));
        }
        if out_members.iter().any(|m| m.kind == "group") {
            score -= 5;
            evidence.push("Includes a shared group container".into());
        }
        if SENSITIVE.iter().any(|s| id.starts_with(s)) {
            score -= 4;
            evidence.push("Browser, mail or chat data persists past uninstall by design".into());
        }
        if score < 4 {
            continue;
        }
        if total < 1_048_576 && out_members.len() < 3 {
            continue;
        }
        let confidence = if score >= 7 { "likely" } else { "confirm" };
        for m in out_members.iter_mut() {
            m.preselect = confidence == "likely" && !m.persistent;
        }
        out_members.sort_by(|a, b| b.total.cmp(&a.total));
        let name = id.rsplit('.').next().unwrap_or(&id).to_string();
        groups.push(RemnantGroup { id: id.clone(), name, score, confidence: confidence.into(), total, newest_mtime: newest, members: out_members, evidence });
    }
    groups.sort_by(|a, b| b.total.cmp(&a.total));

    RemnantsReport {
        schema: 1,
        rules: crate::classify::RULES_VERSION,
        generated_at: now(),
        db: db.map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
        installed_apps: installed.len(),
        groups,
        agents: orphan_agents(home),
    }
}

pub fn write(db: Option<&Path>, out: &Path, home: &Path) -> Result<RemnantsReport, String> {
    let report = build(db, home);
    fs::write(out, serde_json::to_string_pretty(&report).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(report)
}

pub fn read(path: &Path) -> Option<RemnantsReport> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn report_path(reports_dir: &Path) -> PathBuf {
    reports_dir.join("remnants.json")
}
