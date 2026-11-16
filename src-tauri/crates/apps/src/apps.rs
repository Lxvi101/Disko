//! Installed apps and everything that belongs to each of them.
//!
//! Discovery: `.app` bundles in /Applications (with subfolders like Utilities and vendor folders
//! such as "Adobe Photoshop 2026/") and ~/Applications. /System/Applications and symlinked
//! bundles (Safari's Cryptex link) are skipped.
//!
//! Ownership: every data location is matched by exact bundle id (or `<id>.<suffix>` for helpers
//! and extensions), app name, app-group entitlements from the code signature, and launchd jobs
//! whose program lives in the bundle. When a location is claimed by more than one installed app
//! with equal strength (a team group container, a vendor folder), it's *shared*: shown on each
//! app, never removed by an uninstall. The longest bundle id wins otherwise, so
//! `com.google.Chrome.canary` data stays with Canary.
//!
//! Last used (no Spotlight on purpose): newest of the app's Saved Application State, its
//! preference plist(s), its container's preference plist, the own mtime of its id-named cache and
//! support folders, "now" if running, and Spotlight's kMDItemLastUsedDate when Spotlight answers.
//! These are written by the app while it runs, so they track use; they can be touched by
//! background helpers and updaters too, so treat it as "at least this recent".

use crate::size::{self, max_opt};
use crate::types::AppInfo;
use crate::util::{self, name_of, s};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where to look. `Env::real()` for this Mac; tests point it at fixtures.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// App folders to scan (subfolders are scanned up to three levels).
    pub app_dirs: Vec<PathBuf>,
    /// Also match /Library (system-wide support, caches, launch daemons, helper tools).
    pub system: bool,
    /// Ask `codesign` for team id, app groups and signing authority.
    pub codesign: bool,
    /// Ask Spotlight for kMDItemLastUsedDate (an extra; it's off on some Macs).
    pub spotlight: bool,
    /// Ask `ps` which apps are running.
    pub processes: bool,
}

impl Env {
    pub fn real() -> Env {
        let home = disko_core::paths::home();
        Env {
            app_dirs: vec![PathBuf::from("/Applications"), home.join("Applications")],
            home,
            system: true,
            codesign: true,
            spotlight: true,
            processes: true,
        }
    }

    pub fn fixture(home: &Path) -> Env {
        Env { home: home.to_path_buf(), app_dirs: vec![home.join("Applications")], system: false, codesign: false, spotlight: false, processes: false }
    }
}

/// A discovered bundle before ownership and sizes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Bundle {
    pub path: PathBuf,
    pub bundle_root: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub name: String,
    pub bundle_name: Option<String>,
    pub id: String,
    pub version: Option<String>,
    pub team: Option<String>,
    pub groups: Vec<String>,
    pub source: String,
}

fn is_uninstaller(name: &str) -> bool {
    let n = name.to_lowercase();
    ["uninstall", "deinstall", "désinstall", "desinstal", "remover"].iter().any(|k| n.contains(k))
}

pub(crate) fn read_info(app: &Path) -> Option<(String, Option<String>, Option<String>)> {
    let v = plist::Value::from_file(app.join("Contents/Info.plist")).ok()?;
    let d = v.as_dictionary()?;
    let id = d.get("CFBundleIdentifier")?.as_string()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let version = d.get("CFBundleShortVersionString").or_else(|| d.get("CFBundleVersion")).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let bname = d.get("CFBundleDisplayName").or_else(|| d.get("CFBundleName")).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some((id, version, bname))
}

/// Find bundles under one app folder. Vendor folders (a direct subfolder whose single real app
/// carries the folder's name) become the app's `bundle_root`; uninstaller apps beside an app
/// become its `uninstaller` and aren't listed themselves.
pub(crate) fn discover(env: &Env) -> Vec<Bundle> {
    let mut out = Vec::new();
    for root in &env.app_dirs {
        if root.starts_with("/System") {
            continue;
        }
        scan(root, 0, root, &mut out);
    }
    // The same bundle can be reached twice (nested app roots); keep the first.
    let mut seen = std::collections::HashSet::new();
    out.retain(|b| seen.insert(b.path.clone()));
    out
}

fn scan(dir: &Path, depth: u8, root: &Path, out: &mut Vec<Bundle>) {
    let entries = util::children(dir);
    let uninstallers: Vec<PathBuf> = entries.iter().filter(|p| is_uninstaller(&name_of(p))).cloned().collect();
    let mut apps = Vec::new();
    for p in &entries {
        let n = name_of(p);
        if n.starts_with('.') || is_uninstaller(&n) {
            continue;
        }
        let Ok(m) = fs::symlink_metadata(p) else { continue };
        if m.file_type().is_symlink() || !m.is_dir() {
            continue;
        }
        if n.ends_with(".app") {
            apps.push(p.clone());
        } else if depth < 3 && !p.join("Contents/Info.plist").exists() {
            scan(p, depth + 1, root, out);
        }
    }
    let is_vendor_folder = depth == 1 && apps.len() == 1 && !name_of(dir).ends_with(".localized");
    for app in apps {
        let Some((id, version, bundle_name)) = read_info(&app) else { continue };
        let name = name_of(&app).trim_end_matches(".app").to_string();
        let folder = name_of(dir).to_lowercase();
        let bundle_root = (is_vendor_folder && (folder.starts_with(&name.to_lowercase()) || name.to_lowercase().starts_with(&folder))).then(|| dir.to_path_buf());
        let _ = root;
        out.push(Bundle {
            path: app,
            bundle_root,
            uninstaller: uninstallers.first().cloned(),
            name,
            bundle_name,
            id,
            version,
            ..Default::default()
        });
    }
}

type SignInfo = (Option<String>, Vec<String>, String);

/// Team id, app groups and signing source, cached per bundle until its Info.plist changes.
fn codesign_info(b: &mut Bundle) {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (i64, SignInfo)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let stamp = size::own_mtime(&b.path.join("Contents/Info.plist")).unwrap_or(0) ^ size::own_mtime(&b.path.join("Contents/_CodeSignature")).unwrap_or(0);
    if let Some((st, (team, groups, source))) = cache.lock().unwrap().get(&b.path) {
        if *st == stamp {
            (b.team, b.groups, b.source) = (team.clone(), groups.clone(), source.clone());
            return;
        }
    }
    codesign_uncached(b);
    cache.lock().unwrap().insert(b.path.clone(), (stamp, (b.team.clone(), b.groups.clone(), b.source.clone())));
}

fn codesign_uncached(b: &mut Bundle) {
    let Some((out, err)) = util::run("/usr/bin/codesign", &["-dvv", "--entitlements", "-", "--xml", &s(&b.path)], Duration::from_secs(8)) else {
        b.source = "other".into();
        return;
    };
    let authorities: Vec<&str> = err.lines().filter_map(|l| l.strip_prefix("Authority=")).collect();
    b.team = err.lines().filter_map(|l| l.strip_prefix("TeamIdentifier=")).map(str::trim).find(|t| *t != "not set").map(str::to_string);
    b.source = match authorities.first() {
        Some(a) if *a == "Software Signing" => "apple",
        Some(a) if a.starts_with("Apple Mac OS Application Signing") => "app-store",
        Some(a) if a.starts_with("Developer ID Application") => "developer-id",
        None if err.contains("not signed") || err.contains("adhoc") || err.contains("Signature=adhoc") => "unsigned",
        _ => "other",
    }
    .into();
    if let Some(start) = out.find("<?xml") {
        if let Ok(v) = plist::Value::from_reader_xml(out[start..].as_bytes()) {
            if let Some(d) = v.as_dictionary() {
                for key in ["com.apple.security.application-groups", "com.apple.security.application-groups.identifiers"] {
                    if let Some(a) = d.get(key).and_then(|v| v.as_array()) {
                        b.groups.extend(a.iter().filter_map(|x| x.as_string()).map(str::to_string));
                    }
                }
                if b.team.is_none() {
                    b.team = d.get("com.apple.developer.team-identifier").and_then(|v| v.as_string()).map(str::to_string);
                }
            }
        }
    }
}

/// How strongly an app claims a data location. Higher wins; equal claims from different apps
/// make it shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Claim {
    /// Vendor folder ("Application Support/Adobe").
    Vendor = 1,
    /// App name ("Application Support/OrbStack").
    Name = 2,
    /// App group from the entitlements or `<team>.<id>` / `group.<id>`.
    Group = 3,
    /// Launchd job whose program is in the bundle or that names the bundle id.
    Launchd = 4,
    /// Bundle id or `<id>.<helper>`: the longer the id, the stronger (added below).
    Id = 5,
}

fn id_match(key: &str, id: &str) -> bool {
    let k = key.to_ascii_lowercase();
    let i = id.to_ascii_lowercase();
    k == i || (k.starts_with(&i) && k.as_bytes().get(i.len()) == Some(&b'.'))
}

const GENERIC_VENDORS: &[&str] = &["github", "electron", "example", "yourcompany", "apple", "google", "microsoft", "jetbrains", "mozilla", "company", "app", "apps", "dev", "io", "com", "org", "net"];

fn vendor_of(id: &str) -> Option<String> {
    let v = id.split('.').nth(1)?.to_ascii_lowercase();
    (v.len() >= 4 && !GENERIC_VENDORS.contains(&v.as_str())).then_some(v)
}

/// Every data location an app *might* own, with claim strength (and id length for `Id`).
fn candidates(env: &Env, b: &Bundle) -> Vec<(PathBuf, Claim, usize)> {
    let lib = env.home.join("Library");
    let mut out: Vec<(PathBuf, Claim, usize)> = Vec::new();
    let names: Vec<String> = [Some(b.name.clone()), b.bundle_name.clone()].into_iter().flatten().filter(|n| n.chars().count() >= 3).map(|n| n.to_lowercase()).collect();
    let idlen = b.id.len();

    // (dir, strip suffix, match names too)
    let mut id_roots: Vec<(PathBuf, &str, bool)> = vec![
        (lib.join("Application Support"), "", true),
        (lib.join("Caches"), "", true),
        (lib.join("Containers"), "", false),
        (lib.join("Preferences"), ".plist", false),
        (lib.join("Preferences/ByHost"), ".plist", false),
        (lib.join("Saved Application State"), ".savedState", false),
        (lib.join("HTTPStorages"), "", false),
        (lib.join("HTTPStorages"), ".binarycookies", false),
        (lib.join("WebKit"), "", false),
        (lib.join("Logs"), "", true),
        (lib.join("Cookies"), ".binarycookies", false),
        (lib.join("Application Scripts"), "", false),
    ];
    if env.system {
        id_roots.push((PathBuf::from("/Library/Application Support"), "", true));
        id_roots.push((PathBuf::from("/Library/Caches"), "", false));
        id_roots.push((PathBuf::from("/Library/Preferences"), ".plist", false));
        id_roots.push((PathBuf::from("/Library/Logs"), "", true));
    }
    let vendor = vendor_of(&b.id);
    for (dir, suffix, by_name) in &id_roots {
        let by_host = dir.ends_with("ByHost");
        for p in util::children(dir) {
            let n = name_of(&p);
            let key = if suffix.is_empty() { n.as_str() } else if let Some(k) = n.strip_suffix(suffix) { k } else { continue };
            // ByHost plists are "<id>.<hardware uuid>.plist".
            let key = if by_host { key.rsplit_once('.').map(|(a, _)| a).unwrap_or(key) } else { key };
            if id_match(key, &b.id) {
                out.push((p.clone(), Claim::Id, idlen));
            } else if *by_name && names.iter().any(|x| *x == n.to_lowercase()) {
                out.push((p.clone(), Claim::Name, 0));
            } else if *by_name && suffix.is_empty() && vendor.as_deref() == Some(n.to_lowercase().as_str()) && dir.ends_with("Application Support") {
                out.push((p.clone(), Claim::Vendor, 0));
            }
        }
    }

    // Group containers: entitlement groups exactly, plus `<TEAM>.<id>` / `group.<id>`.
    for p in util::children(&lib.join("Group Containers")) {
        let n = name_of(&p);
        let by_entitlement = b.groups.iter().any(|g| g.eq_ignore_ascii_case(&n));
        let stripped = n.strip_prefix("group.").unwrap_or(&n);
        let stripped = match (&b.team, stripped.split_once('.')) {
            (Some(t), Some((head, rest))) if head == t => rest,
            _ => stripped,
        };
        if by_entitlement {
            out.push((p, Claim::Group, 0));
        } else if id_match(stripped, &b.id) {
            out.push((p, Claim::Id, idlen));
        }
    }

    // Launchd jobs (and the privileged helper tools they run).
    let mut job_dirs = vec![lib.join("LaunchAgents")];
    if env.system {
        job_dirs.push(PathBuf::from("/Library/LaunchAgents"));
        job_dirs.push(PathBuf::from("/Library/LaunchDaemons"));
    }
    let bundle = b.bundle_root.as_ref().unwrap_or(&b.path);
    for dir in job_dirs {
        for p in util::children(&dir) {
            if !name_of(&p).ends_with(".plist") {
                continue;
            }
            let Some(job) = launchd_job(&p) else { continue };
            let label_is_ours = id_match(&job.label, &b.id);
            let names_us = job.associated.iter().any(|a| a.eq_ignore_ascii_case(&b.id));
            let program_inside = job.program.as_deref().is_some_and(|prog| Path::new(prog).starts_with(bundle));
            if label_is_ours || names_us || program_inside {
                out.push((p.clone(), Claim::Launchd, 0));
                if let Some(prog) = &job.program {
                    let prog = Path::new(prog);
                    if prog.starts_with("/Library/PrivilegedHelperTools") && util::exists(prog) {
                        out.push((prog.to_path_buf(), Claim::Launchd, 0));
                    }
                }
            }
        }
    }
    if env.system {
        for p in util::children(Path::new("/Library/PrivilegedHelperTools")) {
            if id_match(&name_of(&p), &b.id) {
                out.push((p, Claim::Id, idlen));
            }
        }
    }
    out.retain(|(p, _, _)| !disko_core::paths::is_protected(p));
    out
}

pub(crate) struct Job {
    pub label: String,
    pub program: Option<String>,
    pub associated: Vec<String>,
}

pub(crate) fn launchd_job(p: &Path) -> Option<Job> {
    let v = plist::Value::from_file(p).ok()?;
    let d = v.as_dictionary()?;
    let label = d.get("Label").and_then(|v| v.as_string()).unwrap_or("").to_string();
    let program = d
        .get("Program")
        .and_then(|v| v.as_string())
        .map(str::to_string)
        .or_else(|| d.get("ProgramArguments").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_string()).map(str::to_string));
    let associated = match d.get("AssociatedBundleIdentifiers") {
        Some(plist::Value::String(s)) => vec![s.clone()],
        Some(plist::Value::Array(a)) => a.iter().filter_map(|x| x.as_string()).map(str::to_string).collect(),
        _ => vec![],
    };
    Some(Job { label, program, associated })
}

/// Resolve claims across apps: returns, per bundle, (owned paths, shared paths).
fn resolve(claims: &[Vec<(PathBuf, Claim, usize)>]) -> Vec<(Vec<PathBuf>, Vec<PathBuf>)> {
    // path → (best strength, id length, apps at that strength)
    let mut best: HashMap<&PathBuf, ((Claim, usize), Vec<usize>)> = HashMap::new();
    for (i, cs) in claims.iter().enumerate() {
        for (p, c, len) in cs {
            let strength = (*c, *len);
            let e = best.entry(p).or_insert((strength, vec![]));
            if strength > e.0 {
                *e = (strength, vec![i]);
            } else if strength == e.0 && !e.1.contains(&i) {
                e.1.push(i);
            }
        }
    }
    let mut out = vec![(Vec::new(), Vec::new()); claims.len()];
    for (p, (_, apps)) in best {
        if apps.len() == 1 {
            out[apps[0]].0.push(p.clone());
        } else {
            for a in apps {
                out[a].1.push(p.clone());
            }
        }
    }
    for (owned, shared) in out.iter_mut() {
        owned.sort();
        shared.sort();
        // Never list a path inside another listed path.
        let all = owned.clone();
        owned.retain(|p| !all.iter().any(|o| o != p && p.starts_with(o)));
    }
    out
}

fn root_owned(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.uid() == 0).unwrap_or(false)
}

/// Best-effort last use; see the module docs.
fn last_used(env: &Env, b: &Bundle, owned: &[PathBuf]) -> Option<i64> {
    let lib = env.home.join("Library");
    let mut t: Option<i64> = None;
    let state = lib.join("Saved Application State").join(format!("{}.savedState", b.id));
    t = max_opt(t, size::own_mtime(&state));
    for c in util::children(&state) {
        t = max_opt(t, size::own_mtime(&c));
    }
    t = max_opt(t, size::own_mtime(&lib.join("Preferences").join(format!("{}.plist", b.id))));
    t = max_opt(t, size::own_mtime(&lib.join("Containers").join(&b.id).join("Data/Library/Preferences").join(format!("{}.plist", b.id))));
    for p in owned {
        let parent = p.parent().map(name_of).unwrap_or_default();
        if matches!(parent.as_str(), "Caches" | "Application Support" | "HTTPStorages" | "WebKit") && p.starts_with(&lib) {
            t = max_opt(t, size::own_mtime(p));
        }
    }
    t
}

fn spotlight_last_used(paths: &[String]) -> Vec<Option<i64>> {
    let mut args = vec!["-name", "kMDItemLastUsedDate", "-raw"];
    args.extend(paths.iter().map(|p| p.as_str()));
    let Some((out, _)) = util::run("/usr/bin/mdls", &args, Duration::from_secs(5)) else { return vec![None; paths.len()] };
    let vals: Vec<Option<i64>> = out.split('\0').map(|v| if v.trim() == "(null)" { None } else { util::parse_time(v.trim()) }).collect();
    if vals.len() == paths.len() {
        vals
    } else {
        vec![None; paths.len()]
    }
}

/// The installed apps with their data, largest first.
pub fn apps() -> Vec<AppInfo> {
    inventory(&Env::real())
}

pub fn inventory(env: &Env) -> Vec<AppInfo> {
    let mut bundles = discover(env);
    if env.codesign {
        bundles.par_iter_mut().for_each(codesign_info);
    }
    for b in bundles.iter_mut() {
        if b.source.is_empty() {
            b.source = "other".into();
        }
    }
    let claims: Vec<_> = bundles.par_iter().map(|b| candidates(env, b)).collect();
    let resolved = resolve(&claims);
    let procs = if env.processes { util::process_paths() } else { vec![] };
    let paths: Vec<String> = bundles.iter().map(|b| s(&b.path)).collect();
    let spot = if env.spotlight && !paths.is_empty() { spotlight_last_used(&paths) } else { vec![None; paths.len()] };
    let now = util::now();

    let mut apps: Vec<AppInfo> = bundles
        .par_iter()
        .zip(resolved.par_iter())
        .zip(spot.par_iter())
        .map(|((b, (owned, shared)), spot)| {
            let prefix = format!("{}/", s(&b.path));
            let running = procs.iter().any(|p| p.starts_with(&prefix));
            let measured = b.bundle_root.as_ref().unwrap_or(&b.path);
            let data_paths: Vec<String> = owned.iter().map(|p| s(p)).collect();
            let data = size::usage_all(&data_paths);
            let mut last = max_opt(last_used(env, b, owned), *spot);
            if running {
                last = Some(now);
            }
            let adobe = b.id.to_ascii_lowercase().starts_with("com.adobe.");
            let uninstaller = b.uninstaller.clone().or_else(|| {
                adobe.then(|| PathBuf::from("/Applications/Utilities/Adobe Creative Cloud/ACC/Creative Cloud.app")).filter(|p| p.exists())
            });
            let admin = root_owned(measured) || owned.iter().any(|p| root_owned(p));
            AppInfo {
                name: b.name.clone(),
                bundle_id: b.id.clone(),
                path: s(&b.path),
                version: b.version.clone(),
                icon: format!("app:{}", s(&b.path)),
                app_bytes: size::bytes(measured),
                data_bytes: data.bytes,
                data_paths,
                last_used: last,
                running,
                apple: b.source == "apple",
                bundle_root: b.bundle_root.as_ref().map(|p| s(p)),
                shared_paths: shared.iter().map(|p| s(p)).collect(),
                uninstaller: uninstaller.map(|p| s(&p)),
                admin,
                source: b.source.clone(),
                storage: crate::storage::profile(&b.path, &b.id).is_some(),
            }
        })
        .collect();
    apps.sort_by(|a, b| (b.app_bytes + b.data_bytes).cmp(&(a.app_bytes + a.data_bytes)).then(a.name.cmp(&b.name)));
    apps
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::util::test_dir;

    pub(crate) fn make_app(dir: &Path, name: &str, id: &str) -> PathBuf {
        let app = dir.join(format!("{name}.app"));
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        d.insert("CFBundleShortVersionString".into(), "1.2.3".into());
        plist::Value::Dictionary(d).to_file_xml(app.join("Contents/Info.plist")).unwrap();
        fs::write(app.join("Contents/MacOS").join(name), vec![0u8; 20_000]).unwrap();
        app
    }

    fn touch(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![1u8; bytes]).unwrap();
    }

    #[test]
    fn data_paths_by_id_name_group_and_launchd() {
        let home = test_dir("apps");
        let apps_dir = home.join("Applications");
        let foo = make_app(&apps_dir, "Foo", "com.acme.Foo");
        make_app(&apps_dir, "Bar", "com.acme.Bar");
        make_app(&apps_dir, "Foo Canary", "com.acme.Foo.canary");
        // Vendor folder with an uninstaller beside the app.
        let vendor = apps_dir.join("Acme Studio 2026");
        make_app(&vendor, "Acme Studio 2026", "com.acmestudio.Studio");
        fs::create_dir_all(vendor.join("Acme Studio 2026 deinstallieren")).unwrap();
        fs::create_dir_all(vendor.join("Presets")).unwrap();

        let lib = home.join("Library");
        for p in [
            "Application Support/com.acme.Foo/db",
            "Application Support/Foo/state",
            "Caches/com.acme.Foo/c",
            "Caches/com.acme.Foo.helper/c",
            "Caches/com.acme.Foo.canary/c",
            "Containers/com.acme.Foo.ShareExtension/x",
            "Saved Application State/com.acme.Foo.savedState/window",
            "HTTPStorages/com.acme.Foo/h",
            "WebKit/com.acme.Foo/w",
            "Logs/Foo/log.txt",
            "Group Containers/ABCDE12345.com.acme.Foo/g",
            "Group Containers/ABCDE12345.com.acme.shared/g",
            "Application Support/com.unrelated.Thing/x",
            "Application Support/Acmestudio/shared",
        ] {
            touch(&lib.join(p), 5000);
        }
        touch(&lib.join("Preferences/com.acme.Foo.plist"), 100);
        touch(&lib.join("Preferences/ByHost/com.acme.Foo.0A1B2C3D-0000-1111-2222-333344445555.plist"), 100);
        touch(&lib.join("Cookies/com.acme.Foo.binarycookies"), 100);
        // A launch agent whose program lives inside Foo.app.
        let mut job = plist::Dictionary::new();
        job.insert("Label".into(), "com.othervendor.updater".into());
        job.insert("ProgramArguments".into(), plist::Value::Array(vec![s(&foo.join("Contents/MacOS/Foo")).into()]));
        fs::create_dir_all(lib.join("LaunchAgents")).unwrap();
        plist::Value::Dictionary(job).to_file_xml(lib.join("LaunchAgents/com.othervendor.updater.plist")).unwrap();

        let env = Env::fixture(&home);
        let mut bundles = discover(&env);
        // Simulate the entitlement both acme apps carry.
        for b in bundles.iter_mut() {
            b.team = Some("ABCDE12345".into());
            if b.id.starts_with("com.acme.") {
                b.groups = vec!["ABCDE12345.com.acme.shared".into()];
            }
        }
        let claims: Vec<_> = bundles.iter().map(|b| candidates(&env, b)).collect();
        let resolved = resolve(&claims);
        let idx = |id: &str| bundles.iter().position(|b| b.id == id).unwrap();
        let (owned, shared) = &resolved[idx("com.acme.Foo")];
        let rel: Vec<String> = owned.iter().map(|p| s(p.strip_prefix(&lib).unwrap_or(p))).collect();
        for want in [
            "Application Support/com.acme.Foo",
            "Application Support/Foo",
            "Caches/com.acme.Foo",
            "Caches/com.acme.Foo.helper",
            "Containers/com.acme.Foo.ShareExtension",
            "Saved Application State/com.acme.Foo.savedState",
            "HTTPStorages/com.acme.Foo",
            "WebKit/com.acme.Foo",
            "Logs/Foo",
            "Group Containers/ABCDE12345.com.acme.Foo",
            "Preferences/com.acme.Foo.plist",
            "Preferences/ByHost/com.acme.Foo.0A1B2C3D-0000-1111-2222-333344445555.plist",
            "Cookies/com.acme.Foo.binarycookies",
            "LaunchAgents/com.othervendor.updater.plist",
        ] {
            assert!(rel.contains(&want.to_string()), "missing {want}: {rel:?}");
        }
        // Canary's cache stays with Canary; unrelated data with nobody.
        assert!(!rel.iter().any(|r| r.contains("canary") || r.contains("unrelated")));
        assert!(resolved[idx("com.acme.Foo.canary")].0.iter().any(|p| p.ends_with("Caches/com.acme.Foo.canary")));
        // The group container both acme apps are entitled to is shared, owned by neither.
        assert!(shared.iter().any(|p| p.ends_with("ABCDE12345.com.acme.shared")));
        assert!(resolved[idx("com.acme.Bar")].1.iter().any(|p| p.ends_with("ABCDE12345.com.acme.shared")));
        assert!(!owned.iter().any(|p| p.ends_with("ABCDE12345.com.acme.shared")));
        // Vendor folder app: bundle root + uninstaller, and its vendor support folder.
        let studio = &bundles[idx("com.acmestudio.Studio")];
        assert_eq!(studio.bundle_root.as_deref(), Some(vendor.as_path()));
        assert!(studio.uninstaller.as_ref().is_some_and(|u| name_of(u).contains("deinstallieren")));
        assert!(resolved[idx("com.acmestudio.Studio")].0.iter().any(|p| p.ends_with("Application Support/Acmestudio")));
        // The uninstaller isn't an app of its own.
        assert_eq!(bundles.len(), 4);

        let list = inventory(&env);
        let foo_info = list.iter().find(|a| a.bundle_id == "com.acme.Foo").unwrap();
        assert!(foo_info.app_bytes >= 20_000);
        assert!(foo_info.data_bytes >= 14 * 4096);
        assert_eq!(foo_info.version.as_deref(), Some("1.2.3"));
        assert_eq!(foo_info.icon, format!("app:{}", s(&foo)));
        assert!(foo_info.last_used.is_some());
        assert!(!foo_info.running && !foo_info.apple);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn helpers() {
        assert!(id_match("com.a.B.helper", "com.a.B"));
        assert!(id_match("COM.A.b", "com.a.B"));
        assert!(!id_match("com.a.Bx", "com.a.B"));
        assert!(is_uninstaller("Uninstall Adobe Creative Cloud"));
        assert!(is_uninstaller("Adobe Photoshop 2026 deinstallieren"));
        assert_eq!(vendor_of("com.adobe.Photoshop").as_deref(), Some("adobe"));
        assert_eq!(vendor_of("com.github.foo"), None);
    }
}
