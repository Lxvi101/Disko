//! App caches, logs, the Trash, installers in Downloads, staged updates and Apple's own
//! downloadable extras.

use super::packages::HANDLED_CACHES;
use super::{fixed, MIN};
use crate::apps::is_bundle_id;
use crate::finding::{make, FindingExt};
use crate::procs::who;
use crate::rules::{file_ext_matches, FILE_RULES};
use crate::util::{ago, child_dirs, children, du, du_all, exists, is_dir, is_file, mtime, name_of, now, plist_dict, plist_str, run, s, Size, DAY};
use crate::Clutter;
use disko_core::{Category, Finding, Matched, Safety};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Apple caches in ~/Library/Caches that regrow harmlessly (OnyX clears the same ones).
pub const APPLE_CACHES_OK: &[&str] = &[
    "com.apple.helpd",
    "com.apple.python",
    "com.apple.dt.Xcode",
    "com.apple.dt.xcodebuild",
    "com.apple.preferencepanes.usercache",
    "com.apple.preferencepanes.searchindexcache",
    "com.apple.systemsettings.menucache",
    "com.apple.systempreferences.imageCache",
    "com.apple.tiswitcher.cache",
    "com.apple.Music",
    "com.apple.iTunes",
];

/// macOS's own state that only looks like a cache (iCloud sync, Maps tiles, Family Sharing).
const SYSTEM_CACHES: &[&str] = &["CloudKit", "GeoServices", "familycircled", "FamilyCircle", "PassKit", "com.crashlytics", "SharedImageCache"];

/// One finding per app cache in ~/Library/Caches.
pub fn caches(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join("Library/Caches");
    let apps = c.apps();
    let procs = c.procs();
    child_dirs(&root)
        .into_par_iter()
        .filter_map(|p| {
            let name = name_of(&p);
            if HANDLED_CACHES.contains(&name.as_str()) || SYSTEM_CACHES.contains(&name.as_str()) || name == "app.disko" || (name.starts_with("com.apple.") && (!APPLE_CACHES_OK.contains(&name.as_str()) || name.starts_with("com.apple.dt."))) {
                return None;
            }
            let shipit = name.ends_with(".ShipIt");
            let id = name.trim_end_matches(".ShipIt");
            let app = apps.owner(id).or_else(|| apps.by_name(&name));
            let running_id = procs.list.iter().any(|x| x.exe.contains(&format!("/{}", id)));
            // A cache of an app that's gone is the leftovers probe's (with the rest of its data).
            if is_bundle_id(id) && app.is_none() && !running_id && !id.starts_with("com.apple.") && !shipit {
                return None;
            }
            let size = du(&p);
            if size.bytes < MIN {
                return None;
            }
            let title = app.map(|a| a.name.clone()).unwrap_or_else(|| crate::util::pretty_bundle(id));
            let icon = app.map(|a| format!("app:{}", a.path.display())).unwrap_or_else(|| "app".into());
            let busy = app.and_then(|a| procs.app_running(&a.path)).map(|x| format!("{} is running", who(x)));
            let f = if shipit {
                make("installers.shipit", Category::Installers, title, icon, vec![s(&p)], size, Safety::Safe, false, "An update the app downloaded and already installed (Squirrel ShipIt); the next update downloads fresh.").detail("update download")
            } else {
                let sparkle = exists(&p.join("org.sparkle-project.Sparkle"));
                make("cache.app", Category::Cache, title, icon, vec![s(&p)], size, Safety::Safe, true, format!("Cache the app refills as you use it; last written {}.", ago(size.newest)))
                    .detail(if sparkle { "cache · includes old update downloads".to_string() } else { "cache".to_string() })
            };
            Some(f.keyed(&s(&p)).busy(busy))
        })
        .collect()
}

/// Logs and crash reports: app logs, user and system crash reports, rotated system logs.
pub fn logs(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let apps = c.apps();
    let week = now() - 7 * DAY;
    let mut out = vec![];
    let mut small = vec![];
    for p in children(&home.join("Library/Logs")) {
        let n = name_of(&p);
        if n == "DiagnosticReports" || n == "JetBrains" || n.starts_with('.') {
            continue;
        }
        if is_bundle_id(&n) && apps.owner(&n).is_none() {
            continue; // leftovers
        }
        let size = du(&p);
        if size.bytes == 0 {
            continue;
        }
        if size.bytes >= 20 * MIN {
            let app = apps.owner(&n).or_else(|| apps.by_name(&n));
            let title = app.map(|a| a.name.clone()).unwrap_or_else(|| crate::util::pretty_bundle(&n));
            out.push(make("logs.app", Category::Logs, format!("{} logs", title), "log", vec![s(&p)], size, Safety::Safe, true, format!("Log files, last written {}; only useful while troubleshooting.", ago(size.newest))).keyed(&s(&p)));
        } else {
            small.push(p);
        }
    }
    if !small.is_empty() {
        let size = du_all(&small);
        if size.bytes >= MIN {
            out.push(make("logs.apps", Category::Logs, "App logs", "log", small.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, format!("Logs of {} apps and tools; only useful while troubleshooting.", small.len())).keyed("logs:apps"));
        }
    }
    // Crash reports older than a week (recent ones may matter while troubleshooting).
    for (dir, rule) in [(home.join("Library/Logs/DiagnosticReports"), "logs.crash-reports"), (c.env.sys("/Library/Logs/DiagnosticReports"), "logs.system-crash-reports")] {
        let old: Vec<PathBuf> = walk_files(&dir, 2).into_iter().filter(|p| mtime(p).is_some_and(|t| t < week)).collect();
        if old.is_empty() {
            continue;
        }
        let size = du_all(&old);
        if size.bytes >= MIN / 4 {
            out.push(
                make(rule, Category::Logs, if rule == "logs.crash-reports" { "Crash reports" } else { "System crash reports" }, "log", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, format!("{} crash, hang and diagnostic reports older than a week.", old.len()))
                    .keyed(&s(&dir)),
            );
        }
    }
    // Rotated system logs.
    let var_log = c.env.sys("/private/var/log");
    let mut rotated: Vec<PathBuf> = walk_files(&var_log, 2).into_iter().filter(|p| {
        let n = name_of(p);
        (n.ends_with(".gz") || n.ends_with(".bz2") || (n.ends_with(".asl") && mtime(p).is_some_and(|t| t < week))) && mtime(p).is_some_and(|t| t < week)
    }).collect();
    for p in children(&c.env.sys("/Library/Logs")) {
        let n = name_of(&p);
        if n != "DiagnosticReports" && n != "CrashReporter" && mtime(&p).is_some_and(|t| t < week) && (n.ends_with(".log") || n.ends_with(".old") || n.ends_with(".gz")) {
            rotated.push(p);
        }
    }
    if !rotated.is_empty() {
        let size = du_all(&rotated);
        if size.bytes >= MIN {
            out.push(make("logs.system", Category::Logs, "Old system logs", "log", rotated.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, "Rotated and archived macOS logs older than a week; the live logs stay.").keyed("logs:system"));
        }
    }
    out
}

fn walk_files(dir: &Path, depth: u8) -> Vec<PathBuf> {
    let mut out = vec![];
    for p in children(dir) {
        if is_file(&p) {
            out.push(p);
        } else if depth > 0 && is_dir(&p) {
            out.extend(walk_files(&p, depth - 1));
        }
    }
    out
}

/// The Trash. Deleting from it is final, so it is Review even though the action is Delete.
pub fn trash(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join(".Trash");
    let items = children(&root);
    if items.is_empty() {
        return vec![];
    }
    let size = du_all(&items);
    if size.bytes == 0 {
        return vec![];
    }
    let mut f = make("trash", Category::Trash, "Trash", "trash", items.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("{} items you already threw away. Emptying is final.", items.len())).keyed("trash");
    f.action = disko_core::Action::Delete;
    vec![f]
}

/// Installers and unfinished downloads in Downloads and on the Desktop, at scan start (the walk
/// finds the same ones with the same ids).
pub fn downloads(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let mut out = vec![];
    for root in [home.join("Downloads"), home.join("Desktop")] {
        if !c.env.readable(&root) {
            continue;
        }
        let mut stack = vec![(root, 2u8)];
        while let Some((dir, depth)) = stack.pop() {
            for p in children(&dir) {
                let n = name_of(&p);
                let Ok(m) = fs::symlink_metadata(&p) else { continue };
                let partial_bundle = m.is_dir() && n.ends_with(".download") && mtime(&p).is_some_and(|t| now() - t > 2 * DAY);
                if m.is_dir() && !partial_bundle {
                    if depth > 0 && !n.ends_with(".app") && !n.starts_with('.') {
                        stack.push((p, depth - 1));
                    }
                    continue;
                }
                for (i, r) in FILE_RULES.iter().enumerate() {
                    let hit = match r.id {
                        "installers.file" => m.is_file() && file_ext_matches(r, &n),
                        "installers.partial" => (partial_bundle || file_ext_matches(r, &n)) && mtime(&p).is_some_and(|t| now() - t > 2 * DAY),
                        _ => false,
                    };
                    if hit {
                        let size = if m.is_dir() { du(&p) } else { Size { bytes: std::os::unix::fs::MetadataExt::blocks(&m) * 512, files: 1, newest: mtime(&p) } };
                        if let Some(f) = c.file_match(i, &Matched { tag: 0, path: s(&p), bytes: size.bytes, files: size.files, newest: size.newest }) {
                            out.push(f);
                        }
                        break;
                    }
                }
            }
        }
    }
    out
}

pub fn probe(c: &Clutter) -> Vec<Finding> {
    let jobs: Vec<fn(&Clutter) -> Vec<Finding>> = vec![staged_updates, backups, apple_extras, code_sign_clones];
    jobs.par_iter().flat_map(|j| j(c)).collect()
}

/// Software Update downloads staged in /Library/Updates.
fn staged_updates(c: &Clutter) -> Vec<Finding> {
    let root = c.env.sys("/Library/Updates");
    let mut out = vec![];
    for p in child_dirs(&root) {
        let n = name_of(&p);
        let size = du(&p);
        if size.bytes < MIN {
            continue;
        }
        // Package versions the update would install, vs. what is installed.
        let dist = children(&p).into_iter().find(|f| name_of(f).ends_with(".dist"));
        let text = dist.as_ref().and_then(|d| fs::read_to_string(d).ok()).unwrap_or_default();
        let mut staged: Vec<(String, String)> = vec![];
        for part in text.split("<pkg-ref ").skip(1) {
            let attr = |k: &str| part.split(&format!("{}=\"", k)).nth(1).and_then(|r| r.split('"').next()).map(String::from);
            if let (Some(id), Some(v)) = (attr("id"), attr("version")) {
                if !staged.iter().any(|x| x.0 == id) {
                    staged.push((id, v));
                }
            }
        }
        let mut installed_all = !staged.is_empty();
        let mut installed_v = None;
        let mut staged_v = None;
        for (id, v) in staged.iter().take(3) {
            let info = if c.env.tools { run(Path::new("/usr/sbin/pkgutil"), &["--pkg-info", id], &c.env.home, Duration::from_secs(5)) } else { None };
            let have = info.as_deref().and_then(|t| t.lines().find_map(|l| l.strip_prefix("version: ")).map(|x| x.trim().to_string()));
            staged_v = staged_v.or(Some(v.clone()));
            match have {
                Some(h) if crate::util::parse_version(&h) >= crate::util::parse_version(v) => installed_v = installed_v.or(Some(h)),
                Some(h) => {
                    installed_all = false;
                    installed_v = installed_v.or(Some(h));
                }
                None => installed_all = false,
            }
        }
        let product = if staged.iter().any(|x| x.0.contains("CLTools")) {
            "Command Line Tools update".to_string()
        } else if n == "Rosetta" || staged.iter().any(|x| x.0.contains("Rosetta")) {
            "Rosetta update".to_string()
        } else {
            format!("Software update {}", n)
        };
        let short_v = |v: &str| crate::util::parse_version(v).map(|v| crate::util::version_str(&v[..v.len().min(2)])).unwrap_or_default();
        let why = match (installed_all, &staged_v, &installed_v) {
            (true, Some(sv), _) => format!("Version {} is already installed; Software Update kept the download.", short_v(sv)),
            (false, Some(sv), Some(iv)) => format!("Downloaded {} but never installed (you have {}). Software Update fetches it again when you install.", short_v(sv), short_v(iv)),
            _ => format!("Staged by Software Update {}; it downloads again if needed.", ago(size.newest)),
        };
        let mut f = make("installers.staged-update", Category::Installers, product, "box", vec![s(&p)], size, Safety::Review, false, why);
        f.admin = true;
        out.push(f);
    }
    out
}

/// iPhone/iPad backups not updated for six months.
fn backups(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join("Library/Application Support/MobileSync/Backup");
    if !c.env.readable(&root) {
        return vec![];
    }
    child_dirs(&root)
        .into_par_iter()
        .filter_map(|p| {
            let info = plist_dict(&p.join("Info.plist"));
            let name = info.as_ref().and_then(|d| plist_str(d, "Device Name")).unwrap_or_else(|| "iPhone or iPad".into());
            let size = du(&p);
            let last = size.newest;
            if size.bytes < 10 * MIN || last.is_some_and(|t| now() - t < 180 * DAY) {
                return None;
            }
            Some(
                make("forgotten.device-backup", Category::Forgotten, format!("{} backup", name), "box", vec![s(&p)], size, Safety::Review, false, format!("A local device backup last updated {}. Keep it if it's the only copy of that device's data.", ago(last)))
                    .detail("Finder device backup"),
            )
        })
        .collect()
}

/// Apple's optional downloads and upgrade leftovers.
fn apple_extras(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    if !c.env.fda {
        return out;
    }
    if let Some(f) = fixed(c, "cache.mail-downloads", Category::Cache, "Mail attachments", "app", &["Library/Containers/com.apple.mail/Data/Library/Mail Downloads"], Safety::Safe, true, "Copies of attachments you opened from Mail; the originals stay in your mail.", MIN) {
        out.push(f);
    }
    if let Some(f) = fixed(c, "extras.aerials", Category::Extras, "Aerial wallpapers", "box", &["Library/Application Support/com.apple.wallpaper/aerials/videos", "/Library/Application Support/com.apple.idleassetsd/Customer"], Safety::Review, false, "Downloaded aerial videos for wallpapers and screen savers; macOS downloads the ones you pick again.", 50 * MIN) {
        out.push(f);
    }
    if let Some(f) = fixed(c, "leftovers.previous-libraries", Category::Leftovers, "Previous Music & TV libraries", "box", &["Music/Music/Previous Libraries", "Music/iTunes/Previous iTunes Libraries", "Movies/TV/Previous Libraries"], Safety::Review, false, "Copies macOS made of old library databases during upgrades; your current library doesn't use them.", MIN / 4) {
        out.push(f);
    }
    out
}

/// Chromium-based browsers copy themselves to a temp folder to keep running through updates.
fn code_sign_clones(c: &Clutter) -> Vec<Finding> {
    if !c.env.tools {
        return vec![];
    }
    let Some(t) = run(Path::new("/usr/bin/getconf"), &["DARWIN_USER_TEMP_DIR"], &c.env.home, Duration::from_secs(5)) else { return vec![] };
    let x = Path::new(t.trim().trim_end_matches('/')).parent().map(|p| p.join("X")).unwrap_or_default();
    // /var is a link to /private/var; cleaning refuses paths reached through links.
    let x = fs::canonicalize(&x).unwrap_or(x);
    let apps = c.apps();
    let procs = c.procs();
    children(&x)
        .into_iter()
        .filter(|p| name_of(p).ends_with(".code_sign_clone"))
        .filter_map(|p| {
            let id = name_of(&p).trim_end_matches(".code_sign_clone").to_string();
            let size = du(&p);
            if size.bytes < MIN {
                return None;
            }
            let app = apps.owner(&id);
            let title = app.map(|a| a.name.clone()).unwrap_or_else(|| crate::util::pretty_bundle(&id));
            let busy = app.and_then(|a| procs.app_running(&a.path)).map(|x| format!("{} is running", who(x)));
            Some(
                make("cache.code-sign-clone", Category::Cache, title, app.map(|a| format!("app:{}", a.path.display())).unwrap_or_else(|| "app".into()), vec![s(&p)], size, Safety::Safe, true, "A copy the browser makes of itself so it keeps running through updates. It's an APFS clone, so the real saving is usually much smaller than the size shown; the browser removes it when it quits cleanly.")
                    .detail("code-sign clone")
                    .busy(busy),
            )
        })
        .collect()
}
