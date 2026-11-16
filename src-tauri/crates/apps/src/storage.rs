//! Per-app storage: the full inventory of what one app keeps that you may want to sort out item
//! by item (every simulator, every runtime, every cache folder), as opposed to the clutter
//! crate's filtered findings ("simulators unused for six months").
//!
//! Every item is a `disko_core::Finding`, so removing it goes through the app's one clean path:
//! keep list, protected-path guard, busy checks, the owning tool's command (`xcrun simctl`),
//! Trash + Undo for anything that isn't rebuildable.
//!
//! Profiles are data. Xcode and Adobe have their own builders; Chromium browsers are rows of
//! [`BROWSERS`]; Electron apps are recognised by their framework; coding-agent CLIs that ship
//! with an app are rows of [`CLIS`]. Adding an app usually means adding a row.
//!
//! Never offered: presets, templates, LUTs, licensing and sign-in state, preferences of the
//! installed version, browser history/cookies/profiles, anything outside the listed folders.

use crate::apps::read_info;
use crate::size::{self, Usage};
use crate::util::{self, name_of, s};
use disko_core::{finding_id, Action, Category, Finding, Safety, Source};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------- shapes ----------

/// Everything one app keeps, grouped, for the app's detail page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppStorage {
    pub app: StorageApp,
    /// Sum of all items.
    pub bytes: u64,
    /// Groups with at least one item, in a fixed order per profile.
    pub groups: Vec<StorageGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageApp {
    pub name: String,
    pub path: String,
    /// "app:<path>"
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// "xcode" | "adobe" | "browser" | "electron"
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageGroup {
    /// "simulators" | "runtimes" | "device-support" | "derived-data" | "archives" | "caches" |
    /// "xcodes" | "media-cache" | "old-versions" | "profiles" | "cli-versions"
    pub id: String,
    pub title: String,
    /// Glyph key for the UI.
    pub icon: String,
    pub bytes: u64,
    /// Fine print for the whole group, only where it matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Largest first (simulators: running ones first, then largest).
    pub items: Vec<Finding>,
}

/// Where to look. `StorageEnv::real()` for this Mac; tests point it at fixtures.
#[derive(Debug, Clone)]
pub struct StorageEnv {
    pub home: PathBuf,
    /// "/Applications" (other Xcode copies, installed Adobe apps).
    pub apps_dir: PathBuf,
    /// Run `xcrun simctl` and `xcode-select`.
    pub tools: bool,
    /// Ask `ps` what's running.
    pub processes: bool,
    /// Tests: `(simctl list devices -j, simctl runtime list -j)` output instead of running simctl.
    pub simctl: Option<(String, String)>,
}

impl StorageEnv {
    pub fn real() -> StorageEnv {
        StorageEnv { home: disko_core::paths::home(), apps_dir: PathBuf::from("/Applications"), tools: true, processes: true, simctl: None }
    }

    pub fn fixture(home: &Path) -> StorageEnv {
        StorageEnv { home: home.to_path_buf(), apps_dir: home.join("Applications"), tools: false, processes: false, simctl: None }
    }
}

// ---------- profiles ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Xcode,
    Adobe,
    /// Index into [`BROWSERS`].
    Browser(usize),
    Electron,
}

impl Profile {
    fn key(self) -> &'static str {
        match self {
            Profile::Xcode => "xcode",
            Profile::Adobe => "adobe",
            Profile::Browser(_) => "browser",
            Profile::Electron => "electron",
        }
    }
}

/// A Chromium browser: its bundle id and the folders that hold its profiles (support first; the
/// support folder has "Local State" with the profiles' display names).
pub struct Browser {
    pub id: &'static str,
    pub roots: &'static [&'static str],
}

pub const BROWSERS: &[Browser] = &[
    Browser { id: "com.google.Chrome", roots: &["Library/Application Support/Google/Chrome", "Library/Caches/Google/Chrome"] },
    Browser { id: "com.google.Chrome.beta", roots: &["Library/Application Support/Google/Chrome Beta", "Library/Caches/Google/Chrome Beta"] },
    Browser { id: "com.google.Chrome.canary", roots: &["Library/Application Support/Google/Chrome Canary", "Library/Caches/Google/Chrome Canary"] },
    Browser { id: "com.brave.Browser", roots: &["Library/Application Support/BraveSoftware/Brave-Browser", "Library/Caches/BraveSoftware/Brave-Browser"] },
    Browser { id: "com.microsoft.edgemac", roots: &["Library/Application Support/Microsoft Edge", "Library/Caches/Microsoft Edge"] },
    Browser { id: "com.vivaldi.Vivaldi", roots: &["Library/Application Support/Vivaldi", "Library/Caches/Vivaldi"] },
    Browser { id: "org.chromium.Chromium", roots: &["Library/Application Support/Chromium", "Library/Caches/Chromium"] },
    Browser { id: "net.imput.helium", roots: &["Library/Application Support/net.imput.helium", "Library/Caches/net.imput.helium"] },
    Browser { id: "company.thebrowser.Browser", roots: &["Library/Application Support/Arc/User Data", "Library/Caches/Arc/User Data"] },
];

/// Cache folders inside one Chromium profile (or an Electron app's support folder).
const CHROMIUM_CACHES: &[&str] = &["Cache", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache"];
/// Cache folders at the top of a Chromium user-data folder, shared by its profiles.
const CHROMIUM_SHARED: &[&str] = &["GrShaderCache", "ShaderCache", "GraphiteDawnCache"];

/// Rows of an Electron app's caches: title, folders relative to its support folder ("*" = every
/// child folder, for `Partitions`), why.
const ELECTRON_ROWS: &[(&str, &[&str], &str)] = &[
    ("Web cache", &["Cache", "Partitions/*/Cache"], "Downloaded web content; fetched again when needed."),
    ("Code cache", &["Code Cache", "Partitions/*/Code Cache", "CachedData"], "Compiled scripts; compiled again on the next launch."),
    ("GPU caches", &["GPUCache", "DawnGraphiteCache", "DawnWebGPUCache", "Partitions/*/GPUCache"], "Compiled shaders; rebuilt on the next launch."),
    ("Extension downloads", &["CachedExtensionVSIXs"], "Extension packages already installed."),
];

/// A coding-agent CLI that ships with (or beside) an app: every installed version but the one in
/// use and the newest is offered.
pub struct Cli {
    /// Lower-case app name ("claude" for Claude.app).
    pub app: &'static str,
    pub title: &'static str,
    pub versions: &'static str,
    pub launcher: &'static str,
}

pub const CLIS: &[Cli] = &[
    Cli { app: "claude", title: "Claude Code", versions: ".local/share/claude/versions", launcher: ".local/bin/claude" },
    Cli { app: "cursor", title: "Cursor Agent", versions: ".local/share/cursor-agent/versions", launcher: ".local/bin/cursor-agent" },
];

/// Which profile applies to an app, cheaply (one stat at most). `None`: no detail page.
pub fn profile(app: &Path, id: &str) -> Option<Profile> {
    let lid = id.to_ascii_lowercase();
    let name = name_of(app);
    if lid == "com.apple.dt.xcode" {
        return Some(Profile::Xcode);
    }
    if lid.starts_with("com.adobe.") && !name.contains("Creative Cloud") && !lid.contains(".acc.") {
        return Some(Profile::Adobe);
    }
    if let Some(i) = BROWSERS.iter().position(|b| b.id.eq_ignore_ascii_case(id)) {
        return Some(Profile::Browser(i));
    }
    if app.join("Contents/Frameworks/Electron Framework.framework").exists() {
        return Some(Profile::Electron);
    }
    None
}

/// The storage page of the app at `app` on this Mac.
pub fn storage(app: &Path) -> Option<AppStorage> {
    storage_in(&StorageEnv::real(), app)
}

pub fn storage_in(env: &StorageEnv, app: &Path) -> Option<AppStorage> {
    let (id, version, bundle_name) = read_info(app)?;
    let p = profile(app, &id)?;
    let name = name_of(app).trim_end_matches(".app").to_string();
    let ctx = Ctx { env, app, id: &id, name: &name, bundle_name: bundle_name.as_deref(), procs: if env.processes { util::process_paths() } else { vec![] } };
    let mut groups = match p {
        Profile::Xcode => xcode(&ctx),
        Profile::Adobe => adobe(&ctx),
        Profile::Browser(i) => browser(&ctx, &BROWSERS[i]),
        Profile::Electron => electron(&ctx),
    };
    if let Some(cli) = CLIS.iter().find(|c| c.app == name.to_lowercase()) {
        groups.extend(cli_versions(&ctx, cli));
    }
    // Empty folders are noise; every simulator and runtime is listed whatever its size.
    for g in groups.iter_mut().filter(|g| g.id != "simulators" && g.id != "runtimes") {
        g.items.retain(|f| f.bytes > 4096);
    }
    groups.retain(|g| !g.items.is_empty());
    for g in groups.iter_mut() {
        g.bytes = g.items.iter().map(|f| f.bytes).sum();
        if g.id != "simulators" {
            g.items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.title.cmp(&b.title)));
        }
    }
    Some(AppStorage {
        app: StorageApp { name, path: s(app), icon: format!("app:{}", s(app)), version, profile: p.key().into() },
        bytes: groups.iter().map(|g| g.bytes).sum(),
        groups,
    })
}

struct Ctx<'a> {
    env: &'a StorageEnv,
    app: &'a Path,
    id: &'a str,
    name: &'a str,
    bundle_name: Option<&'a str>,
    /// Executable paths of running processes.
    procs: Vec<String>,
}

impl Ctx<'_> {
    fn home(&self, rel: &str) -> PathBuf {
        self.env.home.join(rel)
    }
    fn app_running(&self, app: &Path) -> bool {
        let prefix = format!("{}/", s(app));
        self.procs.iter().any(|p| p.starts_with(&prefix))
    }
}

// ---------- building items ----------

fn root_owned(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.uid() == 0).unwrap_or(false)
}

/// A storage item. Safe → Delete, Review → Trash (override with `.command`).
#[allow(clippy::too_many_arguments)]
fn item(rule: &str, category: Category, title: impl Into<String>, icon: impl Into<String>, paths: Vec<String>, u: Usage, safety: Safety, regrows: bool, why: impl Into<String>) -> Finding {
    Finding {
        id: finding_id(rule, &paths),
        rule: rule.into(),
        category,
        title: title.into(),
        detail: None,
        icon: icon.into(),
        admin: paths.iter().any(|p| root_owned(Path::new(p))),
        paths,
        bytes: u.bytes,
        last_used: u.newest,
        safety,
        action: if safety == Safety::Safe { Action::Delete } else { Action::Trash },
        regrows,
        why: why.into(),
        busy: None,
        source: Source::Builtin,
    }
}

trait ItemExt {
    fn detail(self, d: impl Into<String>) -> Self;
    fn command(self, argv: Vec<String>, label: &str) -> Self;
    fn keyed(self, key: &str) -> Self;
    fn busy(self, b: Option<String>) -> Self;
    fn used(self, t: Option<i64>) -> Self;
}

impl ItemExt for Finding {
    fn detail(mut self, d: impl Into<String>) -> Self {
        let d = d.into();
        self.detail = (!d.is_empty()).then_some(d);
        self
    }
    fn command(mut self, argv: Vec<String>, label: &str) -> Self {
        self.action = Action::Command { argv, label: label.into() };
        self.admin = false;
        self
    }
    fn keyed(mut self, key: &str) -> Self {
        self.id = finding_id(&self.rule, &[key.to_string()]);
        self
    }
    fn busy(mut self, b: Option<String>) -> Self {
        if self.busy.is_none() {
            self.busy = b;
        }
        self
    }
    fn used(mut self, t: Option<i64>) -> Self {
        if t.is_some() {
            self.last_used = t;
        }
        self
    }
}

fn group(id: &str, title: &str, icon: &str, note: Option<&str>, items: Vec<Finding>) -> StorageGroup {
    StorageGroup { id: id.into(), title: title.into(), icon: icon.into(), bytes: 0, note: note.map(str::to_string), items }
}

/// Real folders only (no symlinks), existing.
fn real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.is_dir() && !m.file_type().is_symlink()).unwrap_or(false)
}

/// `base/rel` where a "*" component stands for every real child folder.
fn expand(base: &Path, rel: &str) -> Vec<PathBuf> {
    let mut out = vec![base.to_path_buf()];
    for part in rel.split('/') {
        out = if part == "*" {
            out.iter().flat_map(|d| util::subdirs(d)).filter(|p| real_dir(p)).collect()
        } else {
            out.iter().map(|d| d.join(part)).filter(|p| util::exists(p)).collect()
        };
    }
    out.retain(|p| util::exists(p) && fs::symlink_metadata(p).map(|m| !m.file_type().is_symlink()).unwrap_or(false));
    out
}

fn strs(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| s(p)).collect()
}

/// "1.2.3" / "17.4" → [1,2,3]; None unless it starts with a number.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = v.split(['.', '-']).map_while(|x| x.parse().ok()).collect();
    (!parts.is_empty()).then_some(parts)
}

// ---------- Xcode ----------

/// Where xcrun finds Xcode. When `xcode-select` points at the Command Line Tools, simctl isn't
/// there: point just our commands at this Xcode instead of changing the global choice.
fn developer_dir(env: &StorageEnv, xcode: &Path) -> Option<String> {
    if !env.tools || std::env::var_os("DEVELOPER_DIR").is_some() {
        return None;
    }
    let (sel, _) = util::run("/usr/bin/xcode-select", &["-p"], Duration::from_secs(5))?;
    let dev = xcode.join("Contents/Developer");
    (!sel.contains(".app/") && dev.is_dir()).then(|| s(&dev))
}

/// `xcrun simctl <args>`, through `env DEVELOPER_DIR=…` when needed.
pub fn simctl_argv(dev: Option<&str>, args: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = match dev {
        Some(d) => vec!["/usr/bin/env".into(), format!("DEVELOPER_DIR={d}"), "/usr/bin/xcrun".into()],
        None => vec!["/usr/bin/xcrun".into()],
    };
    v.push("simctl".into());
    v.extend(args.iter().map(|a| a.to_string()));
    v
}

fn simctl_run(dev: Option<&str>, args: &[&str]) -> Option<String> {
    let mut a: Vec<&str> = vec!["simctl"];
    a.extend_from_slice(args);
    let envs: Vec<(&str, &str)> = dev.map(|d| vec![("DEVELOPER_DIR", d)]).unwrap_or_default();
    let (out, _) = util::run_env("/usr/bin/xcrun", &a, &envs, Duration::from_secs(30))?;
    (!out.trim().is_empty()).then_some(out)
}

type SimctlOut = (Option<String>, Option<String>);

fn simctl_cache() -> &'static Mutex<Option<(Instant, Option<String>, SimctlOut)>> {
    static C: OnceLock<Mutex<Option<(Instant, Option<String>, SimctlOut)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Forget cached simctl output (call after removals).
pub fn invalidate() {
    *simctl_cache().lock().unwrap() = None;
}

/// (devices JSON, runtimes JSON), cached for 30 s.
fn simctl_lists(env: &StorageEnv, dev: Option<&str>) -> SimctlOut {
    if let Some((d, r)) = &env.simctl {
        return (Some(d.clone()), Some(r.clone()));
    }
    if !env.tools {
        return (None, None);
    }
    if let Some((at, key, out)) = simctl_cache().lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_secs(30) && key.as_deref() == dev {
            return out.clone();
        }
    }
    let (devices, runtimes) = rayon::join(|| simctl_run(dev, &["list", "devices", "-j"]), || simctl_run(dev, &["runtime", "list", "-j"]));
    let out = (devices, runtimes);
    *simctl_cache().lock().unwrap() = Some((Instant::now(), dev.map(str::to_string), out.clone()));
    out
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sim {
    pub udid: String,
    pub name: String,
    pub runtime: String,
    pub device_type: String,
    pub state: String,
    pub available: bool,
    pub last_used: Option<i64>,
    pub data_size: Option<u64>,
}

/// `simctl list devices -j`.
pub(crate) fn parse_devices(json: &str) -> Option<Vec<Sim>> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut out = vec![];
    for (runtime, list) in v.get("devices")?.as_object()? {
        for d in list.as_array().into_iter().flatten() {
            let Some(udid) = d["udid"].as_str() else { continue };
            out.push(Sim {
                udid: udid.into(),
                name: d["name"].as_str().unwrap_or(udid).into(),
                runtime: runtime.clone(),
                device_type: d["deviceTypeIdentifier"].as_str().unwrap_or("").into(),
                state: d["state"].as_str().unwrap_or("Unknown").into(),
                available: d["isAvailable"].as_bool().unwrap_or(true),
                last_used: d["lastUsedAt"].as_str().or(d["lastBootedAt"].as_str()).and_then(util::parse_time),
                data_size: d["dataPathSize"].as_u64(),
            });
        }
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Runtime {
    pub identifier: String,
    pub runtime_id: String,
    pub platform: String,
    pub version: String,
    pub build: String,
    pub bytes: u64,
    pub last_used: Option<i64>,
    pub deletable: bool,
    pub state: String,
}

/// `simctl runtime list -j`.
pub(crate) fn parse_runtimes(json: &str) -> Option<Vec<Runtime>> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut out: Vec<Runtime> = v
        .as_object()?
        .values()
        .filter_map(|r| {
            Some(Runtime {
                identifier: r["identifier"].as_str()?.into(),
                runtime_id: r["runtimeIdentifier"].as_str().unwrap_or("").into(),
                platform: platform_name(r["platformIdentifier"].as_str().unwrap_or("")).into(),
                version: r["version"].as_str().unwrap_or("").into(),
                build: r["build"].as_str().unwrap_or("").into(),
                bytes: r["sizeBytes"].as_u64().unwrap_or(0),
                last_used: r["lastUsedAt"].as_str().and_then(util::parse_time),
                deletable: r["deletable"].as_bool().unwrap_or(false),
                state: r["state"].as_str().unwrap_or("").into(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    Some(out)
}

fn platform_name(id: &str) -> &'static str {
    let id = id.to_ascii_lowercase();
    if id.contains("iphone") || id.contains("ios") {
        "iOS"
    } else if id.contains("watch") {
        "watchOS"
    } else if id.contains("appletv") || id.contains("tvos") {
        "tvOS"
    } else if id.contains("xr") || id.contains("vision") {
        "visionOS"
    } else {
        "Simulator"
    }
}

/// "com.apple.CoreSimulator.SimRuntime.iOS-27-0" → "iOS 27.0".
pub(crate) fn runtime_label(id: &str) -> String {
    let raw = id.rsplit('.').next().unwrap_or(id);
    let (platform, version) = raw.split_once('-').unwrap_or((raw, ""));
    let platform = match platform {
        "xrOS" => "visionOS",
        p => p,
    };
    format!("{} {}", platform, version.replace('-', ".")).trim().to_string()
}

fn sim_icon(device_type: &str, name: &str) -> &'static str {
    let t = format!("{device_type} {name}").to_ascii_lowercase();
    if t.contains("ipad") {
        "tablet"
    } else if t.contains("watch") {
        "watch"
    } else if t.contains("tv") {
        "tv"
    } else if t.contains("vision") {
        "vision"
    } else {
        "iphone"
    }
}

fn xcode(c: &Ctx) -> Vec<StorageGroup> {
    let dev = developer_dir(c.env, c.app);
    let (devices, runtimes) = simctl_lists(c.env, dev.as_deref());
    let sims = devices.as_deref().and_then(parse_devices);
    let runtimes = runtimes.as_deref().and_then(parse_runtimes).unwrap_or_default();
    let building = c.procs.iter().any(|p| BUILD_TOOLS.contains(&name_of(Path::new(p)).as_str()));
    let mut groups = vec![
        simulators(c, sims.as_deref(), dev.as_deref()),
        sim_runtimes(&runtimes, sims.as_deref().unwrap_or(&[]), dev.as_deref()),
        device_support(c),
        derived_data(c, building),
        archives(c),
        xcode_caches(c),
    ];
    groups.push(other_xcodes(c));
    groups
}

const BUILD_TOOLS: &[&str] = &["xcodebuild", "XCBBuildService", "SWBBuildService", "swift-frontend"];

fn simulators(c: &Ctx, sims: Option<&[Sim]>, dev: Option<&str>) -> StorageGroup {
    let root = c.home("Library/Developer/CoreSimulator/Devices");
    let note = "Each simulator's test apps and their data go with it. Xcode makes new ones whenever you need them.";
    let Some(sims) = sims else {
        // simctl didn't answer: show what's there, but nothing can be removed without it.
        let items: Vec<Finding> = util::subdirs(&root)
            .into_par_iter()
            .filter(|p| is_uuid(&name_of(p)))
            .map(|p| {
                let d = plist::Value::from_file(p.join("device.plist")).ok();
                let get = |k: &str| d.as_ref().and_then(|v| v.as_dictionary()?.get(k)?.as_string().map(str::to_string)).unwrap_or_default();
                let name = Some(get("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| name_of(&p));
                let u = size::usage(&p);
                item("developer.simulator", Category::Developer, name.clone(), sim_icon(&get("deviceType"), &name), vec![s(&p)], u, Safety::Review, false, "A simulator device.")
                    .detail(runtime_label(&get("runtime")))
                    .busy(Some("Couldn't ask the Simulator about it; open Xcode once".into()))
            })
            .collect();
        return group("simulators", "Simulators", "iphone", Some(note), items);
    };
    let mut items: Vec<Finding> = sims
        .par_iter()
        .map(|d| {
            let dir = root.join(&d.udid);
            let u = match d.data_size {
                Some(b) => Usage { bytes: b + size::own_mtime(&dir).map(|_| 4096).unwrap_or(0), newest: d.last_used },
                None => size::usage(&dir),
            };
            let mut detail = runtime_label(&d.runtime);
            if !d.available {
                detail.push_str(" · runtime not installed");
            }
            let why = if d.available { "Its installed test apps and their data go with it." } else { "Its runtime isn't installed any more, so it can never boot again." };
            let busy = match d.state.as_str() {
                "Shutdown" => None,
                "Booted" => Some("Running now; shut it down first".to_string()),
                other => Some(format!("{other} now")),
            };
            item("developer.simulator", Category::Developer, d.name.clone(), sim_icon(&d.device_type, &d.name), vec![s(&dir)], u, Safety::Review, false, why)
                .detail(detail)
                .used(d.last_used)
                .command(simctl_argv(dev, &["delete", &d.udid]), "simctl delete")
                .busy(busy)
        })
        .collect();
    items.sort_by(|a, b| b.busy.is_some().cmp(&a.busy.is_some()).then(b.bytes.cmp(&a.bytes)).then(a.title.cmp(&b.title)));
    group("simulators", "Simulators", "iphone", Some(note), items)
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}

fn sim_runtimes(runtimes: &[Runtime], sims: &[Sim], dev: Option<&str>) -> StorageGroup {
    let items = runtimes
        .iter()
        .map(|r| {
            let users: Vec<&Sim> = sims.iter().filter(|d| !r.runtime_id.is_empty() && d.runtime == r.runtime_id).collect();
            let running = users.iter().find(|d| d.state != "Shutdown");
            let title = format!("{} {}", r.platform, r.version).trim().to_string();
            let detail = match users.len() {
                0 => "Used by no simulator".to_string(),
                1 => "Used by 1 simulator".to_string(),
                n => format!("Used by {n} simulators"),
            };
            let busy = if !r.deletable {
                Some("Part of Xcode; it can't be removed on its own".to_string())
            } else if let Some(d) = running {
                Some(format!("{} is running on it", d.name))
            } else if r.state != "Ready" && !r.state.is_empty() {
                Some(format!("{} now", r.state))
            } else {
                None
            };
            let why = if users.is_empty() {
                format!("{title} simulator runtime (build {}). Xcode › Settings › Components downloads it again.", r.build)
            } else {
                format!("{title} simulator runtime (build {}). Its {} simulators stop working until you download it again in Xcode › Settings › Components.", r.build, users.len())
            };
            // Same rule, key and command as the clutter crate's runtime finding, so both views
            // talk about the same thing.
            let argv = vec!["/usr/bin/xcrun".to_string(), "simctl".into(), "runtime".into(), "delete".into(), r.identifier.clone()];
            let argv = match dev {
                Some(_) => simctl_argv(dev, &["runtime", "delete", &r.identifier]),
                None => argv,
            };
            item("extras.simulator-runtime", Category::Extras, title, "iphone", vec![], Usage { bytes: r.bytes, newest: r.last_used }, Safety::Review, false, why)
                .keyed(&format!("runtime:{}", r.identifier))
                .detail(detail)
                .command(argv, "simctl runtime delete")
                .busy(busy)
        })
        .collect();
    group("runtimes", "Simulator runtimes", "disc", Some("Removing a runtime is permanent; Xcode can download it again."), items)
}

/// "iPhone15,2 17.4 (21E219)" → ("iPhone15,2", "17.4 (21E219)").
pub(crate) fn support_name(name: &str) -> (String, String) {
    match name.split_once(' ') {
        Some((first, rest)) if !first.starts_with(|c: char| c.is_ascii_digit()) => (first.into(), rest.into()),
        _ => (String::new(), name.into()),
    }
}

fn device_support(c: &Ctx) -> StorageGroup {
    let mut dirs = vec![];
    for platform in ["iOS", "watchOS", "tvOS", "visionOS", "macOS"] {
        for p in util::subdirs(&c.home(&format!("Library/Developer/Xcode/{platform} DeviceSupport"))) {
            if real_dir(&p) {
                dirs.push((platform, p));
            }
        }
    }
    let items = dirs
        .into_par_iter()
        .map(|(platform, p)| {
            let (model, version) = support_name(&name_of(&p));
            let u = size::usage(&p);
            let icon = match platform {
                "watchOS" => "watch",
                "tvOS" => "tv",
                "visionOS" => "vision",
                "macOS" => "app",
                _ if model.starts_with("iPad") => "tablet",
                _ => "iphone",
            };
            item("developer.device-support", Category::Developer, format!("{platform} {version}"), icon, vec![s(&p)], u, Safety::Safe, true, "Debug symbols Xcode copied off a device; copied again when that device connects.")
                .detail(model)
                .used(size::own_mtime(&p))
        })
        .collect();
    group("device-support", "Device support", "iphone", None, items)
}

/// "MyApp-abcdefghijklmnopqrstuvwxyz" → "MyApp".
fn derived_name(dir: &str) -> String {
    match dir.rsplit_once('-') {
        Some((n, hash)) if hash.len() >= 20 && hash.chars().all(|c| c.is_ascii_lowercase()) => n.to_string(),
        _ => dir.to_string(),
    }
}

fn derived_data(c: &Ctx, building: bool) -> StorageGroup {
    let items = util::subdirs(&c.home("Library/Developer/Xcode/DerivedData"))
        .into_par_iter()
        .filter(|p| real_dir(p))
        .map(|p| {
            let raw = name_of(&p);
            let u = size::usage(&p);
            let hot = building && u.newest.is_some_and(|t| util::now() - t < 3600);
            let busy = hot.then(|| "Xcode is building".to_string());
            if raw.ends_with(".noindex") {
                return item("developer.derived-data", Category::Developer, "Module cache", "xcode", vec![s(&p)], u, Safety::Safe, true, "Precompiled modules shared by all projects; rebuilt on the next build.").detail(raw).busy(busy);
            }
            let ws = plist::Value::from_file(p.join("info.plist")).ok().and_then(|v| v.as_dictionary()?.get("WorkspacePath")?.as_string().map(str::to_string));
            let title = ws.as_deref().map(|w| name_of(Path::new(w)).trim_end_matches(".xcodeproj").trim_end_matches(".xcworkspace").to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| derived_name(&raw));
            let gone = ws.as_deref().is_some_and(|w| !Path::new(w).exists());
            let detail = match &ws {
                Some(_) if gone => "Project no longer exists".to_string(),
                Some(w) => disko_core::paths::short(Path::new(w).parent().map(|x| s(x)).as_deref().unwrap_or(w.as_str())),
                None => String::new(),
            };
            item("developer.derived-data", Category::Build, title, "xcode", vec![s(&p)], u, Safety::Safe, true, "Build output and index; Xcode rebuilds it on the next build.").detail(detail).busy(busy)
        })
        .collect();
    group("derived-data", "Build data", "xcode", None, items)
}

fn archives(c: &Ctx) -> StorageGroup {
    let mut list = vec![];
    for day in util::subdirs(&c.home("Library/Developer/Xcode/Archives")) {
        for a in util::subdirs(&day) {
            if name_of(&a).ends_with(".xcarchive") && real_dir(&a) {
                list.push(a);
            }
        }
    }
    let items = list
        .into_par_iter()
        .map(|a| {
            let info = plist::Value::from_file(a.join("Info.plist")).ok();
            let d = info.as_ref().and_then(|v| v.as_dictionary());
            let get = |k: &str| d.and_then(|d| d.get(k)?.as_string().map(str::to_string));
            let props = d.and_then(|d| d.get("ApplicationProperties")?.as_dictionary());
            let pget = |k: &str| props.and_then(|p| p.get(k)?.as_string().map(str::to_string));
            let name = get("Name").unwrap_or_else(|| name_of(&a).trim_end_matches(".xcarchive").to_string());
            let version = match (pget("CFBundleShortVersionString"), pget("CFBundleVersion")) {
                (Some(v), Some(b)) => format!("{v} ({b})"),
                (Some(v), None) => v,
                (None, Some(b)) => b,
                _ => String::new(),
            };
            let created = d.and_then(|d| d.get("CreationDate")?.as_date()).and_then(|t| std::time::SystemTime::from(t).duration_since(std::time::UNIX_EPOCH).ok()).map(|x| x.as_secs() as i64);
            let u = size::usage(&a);
            item("developer.archive", Category::Developer, name, "archive", vec![s(&a)], u, Safety::Review, false, "An archived build. Keep the ones whose crash reports you still need to read.")
                .detail(version)
                .used(created)
        })
        .collect();
    group("archives", "Archives", "archive", None, items)
}

/// (relative path, title, category, why)
const XCODE_CACHES: &[(&str, &str, Category, &str)] = &[
    ("Library/Caches/com.apple.dt.Xcode", "Xcode cache", Category::Cache, "Xcode refills it as you work."),
    ("Library/Caches/com.apple.dt.xcodebuild", "xcodebuild cache", Category::Cache, "Refilled by the next command-line build."),
    ("Library/Developer/Xcode/DocumentationCache", "Documentation cache", Category::Cache, "Rebuilt when you open the documentation."),
    ("Library/Developer/Xcode/Products", "Exported products", Category::Developer, "Copies of built products; built again on demand."),
    ("Library/Developer/CoreSimulator/Caches", "Simulator caches", Category::Cache, "The Simulator rebuilds them when it boots a device."),
    ("Library/Developer/XCTestDevices", "Test simulator clones", Category::Developer, "Copies Xcode made for parallel tests; made again on the next run."),
    ("Library/Developer/Xcode/UserData/Previews", "SwiftUI previews", Category::Developer, "Preview simulators and builds; made again when you open a preview."),
    ("Library/Developer/Xcode/iOS Device Logs", "Device logs", Category::Logs, "Console logs Xcode copied off your devices."),
    ("Library/Logs/CoreSimulator", "Simulator logs", Category::Logs, "Logs of simulators."),
    ("Library/Developer/DVTDownloads", "Downloaded components", Category::Installers, "Components Xcode downloaded to install them."),
];

fn xcode_caches(c: &Ctx) -> StorageGroup {
    let items = XCODE_CACHES
        .par_iter()
        .filter_map(|(rel, title, cat, why)| {
            let p = c.home(rel);
            if !real_dir(&p) {
                return None;
            }
            let u = size::usage(&p);
            (u.bytes > 0).then(|| item("developer.cache", *cat, *title, if *cat == Category::Logs { "log" } else { "cache" }, vec![s(&p)], u, Safety::Safe, true, *why))
        })
        .collect();
    group("caches", "Caches", "cache", None, items)
}

fn other_xcodes(c: &Ctx) -> StorageGroup {
    let selected = if c.env.tools { util::run("/usr/bin/xcode-select", &["-p"], Duration::from_secs(5)).map(|x| x.0).unwrap_or_default() } else { String::new() };
    let items = util::children(&c.env.apps_dir)
        .into_iter()
        .filter(|p| {
            let n = name_of(p);
            n.starts_with("Xcode") && n.ends_with(".app") && p.as_path() != c.app && real_dir(p) && !selected.trim().starts_with(&format!("{}/", s(p)))
        })
        .filter_map(|p| {
            let (id, version, _) = read_info(&p)?;
            (id == "com.apple.dt.Xcode").then_some((p, version))
        })
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(p, version)| {
            let u = size::usage(&p);
            let running = c.app_running(&p);
            item("versions.xcode-copy", Category::OldVersions, name_of(&p).trim_end_matches(".app").to_string(), format!("app:{}", s(&p)), vec![s(&p)], u, Safety::Review, false, "Another copy of Xcode.")
                .detail(version.map(|v| format!("Version {v}")).unwrap_or_default())
                .used(size::own_mtime(&p))
                .busy(running.then(|| "Open now".to_string()))
        })
        .collect();
    group("xcodes", "Other Xcode copies", "xcode", None, items)
}

// ---------- Adobe ----------

/// Cache folder names Adobe's video apps share; the only names ever offered (also at custom
/// locations from the preferences): (folder, title, why).
const ADOBE_CACHES: &[(&str, &str, &str)] = &[
    ("Media Cache Files", "Media cache files", "Audio and video Premiere, After Effects and Media Encoder prepared on import; prepared again when needed."),
    ("Media Cache", "Media cache database", "The index of the media cache; rebuilt when needed."),
    ("Peak Files", "Audio waveforms", "Waveform drawings of your audio; drawn again when needed."),
    ("Analyzer Cache Files", "Analyzer cache", "Results of media analysis; made again when needed."),
];

/// An installed Adobe app: product key ("after effects"), display name ("Adobe After Effects"),
/// year (2026) and major version.
#[derive(Debug, Clone)]
struct AdobeApp {
    path: PathBuf,
    name: String,
    product: String,
    display: String,
    year: Option<u32>,
    major: Option<u64>,
}

/// "Adobe After Effects 2026" → ("after effects", Some(2026)); "Adobe Photoshop 2024 Settings"
/// → ("photoshop", Some(2024)); "Premiere Pro" → ("premiere pro", None).
pub(crate) fn product_key(name: &str) -> (String, Option<u32>) {
    let n = name.trim().trim_end_matches(".app");
    let n = n.strip_suffix(" Settings").unwrap_or(n);
    let n = n.strip_prefix("Adobe ").unwrap_or(n);
    let (base, year) = match n.rsplit_once(' ') {
        Some((b, y)) if y.len() == 4 && y.starts_with("20") && y.chars().all(|c| c.is_ascii_digit()) => (b, y.parse().ok()),
        _ => (n, None),
    };
    (base.trim().to_lowercase(), year)
}

fn adobe_apps(apps_dir: &Path) -> Vec<AdobeApp> {
    let mut out = vec![];
    for p in util::children(apps_dir) {
        let n = name_of(&p);
        if !n.starts_with("Adobe ") || n.contains("Creative Cloud") {
            continue;
        }
        let bundles = if n.ends_with(".app") { vec![p.clone()] } else { util::children(&p).into_iter().filter(|x| name_of(x).ends_with(".app") && !name_of(x).to_lowercase().contains("uninstall")).collect() };
        for b in bundles {
            let Some((id, version, _)) = read_info(&b) else { continue };
            if !id.to_lowercase().starts_with("com.adobe.") {
                continue;
            }
            let name = name_of(&b).trim_end_matches(".app").to_string();
            let (product, year) = product_key(&name);
            let display = match year {
                Some(y) => name.trim_end_matches(&format!(" {y}")).to_string(),
                None => name.clone(),
            };
            out.push(AdobeApp { path: b, name, product, display, year, major: version.as_deref().and_then(parse_version).and_then(|v| v.first().copied()) });
        }
    }
    out
}

fn adobe_running(apps: &[AdobeApp], procs: &[String]) -> Option<String> {
    apps.iter().find(|a| {
        let prefix = format!("{}/", s(&a.path));
        procs.iter().any(|p| p.starts_with(&prefix))
    }).map(|a| format!("{} is open; quit it first", a.name))
}

/// Key/value pairs `"key" = "value"` in an After Effects preferences text file.
fn ae_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(i) = rest.find("\" = \"") {
        let before = &rest[..i];
        let key = before.rfind('"').map(|k| &before[k + 1..]).unwrap_or("");
        let after = &rest[i + 5..];
        let Some(end) = after.find('"') else { break };
        out.push((key.to_string(), after[..end].to_string()));
        rest = &after[end..];
    }
    out
}

/// Cache folders the user moved elsewhere in Premiere/Media Encoder or After Effects preferences.
fn adobe_custom_roots(c: &Ctx) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut media = vec![];
    let mut disk = vec![];
    // Premiere Pro / Media Encoder: ~/Documents/Adobe/<app>/<version>/Profile-<user>/<…> Prefs (XML).
    for app in util::subdirs(&c.home("Documents/Adobe")) {
        for ver in util::subdirs(&app) {
            for prof in util::subdirs(&ver).into_iter().filter(|p| name_of(p).starts_with("Profile-")) {
                for f in util::children(&prof).into_iter().filter(|f| name_of(f).ends_with("Prefs") && f.is_file()) {
                    let text = String::from_utf8_lossy(&util::head(&f, 8 << 20)).to_string();
                    let mut rest = text.as_str();
                    while let Some(i) = rest.find("<BE.Prefs.MediaCache.") {
                        let tag = &rest[i + 1..];
                        let Some(close) = tag.find('>') else { break };
                        let value = &tag[close + 1..];
                        let end = value.find('<').unwrap_or(0);
                        let v = value[..end].trim();
                        if v.starts_with('/') {
                            media.push(PathBuf::from(v));
                        }
                        rest = &tag[close..];
                    }
                }
            }
        }
    }
    // After Effects: ~/Library/Preferences/Adobe/After Effects/<version>/*.txt
    for ver in util::subdirs(&c.home("Library/Preferences/Adobe/After Effects")) {
        for f in util::children(&ver).into_iter().filter(|f| name_of(f).ends_with(".txt")) {
            let text = String::from_utf8_lossy(&util::head(&f, 4 << 20)).to_string();
            for (k, v) in ae_pairs(&text) {
                let k = k.to_lowercase();
                if k.contains("disk cache") && (k.contains("folder") || k.contains("path") || k.contains("location")) && v.starts_with('/') {
                    disk.push(PathBuf::from(v));
                }
            }
        }
    }
    (media, disk)
}

fn adobe(c: &Ctx) -> Vec<StorageGroup> {
    let apps = adobe_apps(&c.env.apps_dir);
    let busy = adobe_running(&apps, &c.procs);
    let default_common = c.home("Library/Application Support/Adobe/Common");
    let (media_roots, disk_roots) = adobe_custom_roots(c);

    // Media cache: default place plus custom ones, only folders with the known names.
    let mut found: Vec<(PathBuf, &str, &str, bool)> = vec![]; // (path, title, why, custom)
    let mut roots = vec![(default_common.clone(), false)];
    roots.extend(media_roots.into_iter().map(|r| (r, true)));
    for (root, custom) in roots {
        for &(folder, title, why) in ADOBE_CACHES {
            let cand = if name_of(&root) == folder { root.clone() } else { root.join(folder) };
            if real_dir(&cand) && !found.iter().any(|f| f.0 == cand) {
                found.push((cand, title, why, custom));
            }
        }
    }
    let mut media: Vec<Finding> = found
        .into_par_iter()
        .filter(|(p, _, _, _)| !disko_core::paths::is_protected(p))
        .map(|(p, title, why, custom)| {
            let u = size::usage(&p);
            item("cache.adobe-media", Category::Cache, title, "film", vec![s(&p)], u, Safety::Safe, true, why)
                .detail(if custom { disko_core::paths::short(&s(&p)) } else { String::new() })
                .busy(busy.clone())
        })
        .collect();
    // After Effects disk cache: ~/Library/Caches/Adobe/After Effects/<version>/Disk Cache*, plus custom.
    let mut disk: Vec<(PathBuf, String)> = vec![];
    for ver in util::subdirs(&c.home("Library/Caches/Adobe/After Effects")) {
        for d in util::subdirs(&ver) {
            if name_of(&d).starts_with("Disk Cache") {
                disk.push((d, name_of(&ver)));
            }
        }
    }
    for r in disk_roots {
        if name_of(&r).starts_with("Disk Cache") {
            disk.push((r, String::new()));
        } else {
            disk.extend(util::subdirs(&r).into_iter().filter(|d| name_of(d).starts_with("Disk Cache")).map(|d| (d, String::new())));
        }
    }
    let mut seen = HashSet::new();
    disk.retain(|(p, _)| real_dir(p) && !disko_core::paths::is_protected(p) && seen.insert(p.clone()));
    media.extend(disk.into_par_iter().map(|(p, ver)| {
        let u = size::usage(&p);
        let detail = if ver.is_empty() { disko_core::paths::short(&s(&p)) } else { format!("Version {ver}") };
        item("cache.adobe-disk-cache", Category::Cache, "After Effects disk cache", "film", vec![s(&p)], u, Safety::Safe, true, "Rendered frames After Effects keeps to play back faster; rendered again when needed.").detail(detail).busy(busy.clone())
    }).collect::<Vec<_>>());

    vec![
        group("media-cache", "Media caches", "film", Some("Adobe's apps make these again when they need them."), media),
        group("old-versions", "Data of older versions", "archive", None, adobe_old_versions(c, &apps)),
    ]
}

/// Support, cache and preference folders of Adobe versions older than the installed one of the
/// same product: year-named ("Adobe Photoshop 2024" beside Photoshop 2026) or numbered
/// ("After Effects/24.0" beside After Effects 26.x). Only older ones, never the installed version.
fn adobe_old_versions(c: &Ctx, apps: &[AdobeApp]) -> Vec<Finding> {
    if apps.is_empty() {
        return vec![];
    }
    let installed = |product: &str| -> Vec<&AdobeApp> { apps.iter().filter(|a| a.product == product).collect() };
    // (display title) → (paths, app icon)
    let mut old: BTreeMap<String, (Vec<PathBuf>, String)> = BTreeMap::new();
    let mut add = |title: String, p: PathBuf, icon: &Path| {
        let e = old.entry(title).or_insert_with(|| (vec![], format!("app:{}", s(icon))));
        e.0.push(p);
    };
    let year_roots = ["Library/Application Support/Adobe", "Library/Preferences/Adobe", "Library/Caches/Adobe", "Library/Preferences"];
    for rel in year_roots {
        let only_adobe_names = rel == "Library/Preferences";
        for p in util::children(&c.home(rel)) {
            let n = name_of(&p);
            if (only_adobe_names && !n.starts_with("Adobe ")) || !real_dir(&p) {
                continue;
            }
            let (product, year) = product_key(&n);
            let Some(year) = year else { continue };
            let same = installed(&product);
            let newest = same.iter().filter_map(|a| a.year).max();
            if newest.is_some_and(|ny| year < ny) && !same.iter().any(|a| a.year == Some(year)) {
                let a = same[0];
                add(format!("{} {}", a.display, year), p, &a.path);
            }
        }
    }
    for rel in ["Library/Application Support/Adobe", "Library/Preferences/Adobe", "Library/Caches/Adobe"] {
        for product_dir in util::subdirs(&c.home(rel)) {
            let (product, year) = product_key(&name_of(&product_dir));
            if year.is_some() {
                continue;
            }
            let same = installed(&product);
            let Some(major) = same.iter().filter_map(|a| a.major).max() else { continue };
            for v in util::subdirs(&product_dir) {
                let n = name_of(&v);
                let Some(ver) = parse_version(&n) else { continue };
                if !n.starts_with(|ch: char| ch.is_ascii_digit()) || !real_dir(&v) {
                    continue;
                }
                if ver[0] < major {
                    let a = same[0];
                    add(format!("{} {}", a.display, n), v, &a.path);
                }
            }
        }
    }
    old.into_iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(title, (paths, icon))| {
            let u = size::usage_all(&strs(&paths));
            let mut where_: Vec<&str> = paths
                .iter()
                .map(|p| {
                    let x = s(p);
                    if x.contains("/Preferences/") {
                        "settings"
                    } else if x.contains("/Caches/") {
                        "cache"
                    } else {
                        "support files"
                    }
                })
                .collect();
            where_.sort();
            where_.dedup();
            item("versions.adobe-data", Category::OldVersions, title, icon, strs(&paths), u, Safety::Review, false, "Left behind by an older version; the installed version has its own.").detail(where_.join(", "))
        })
        .collect()
}

// ---------- Chromium browsers and Electron apps ----------

fn chromium_profile_names(support: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(support.join("Local State")) else { return out };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return out };
    if let Some(cache) = v["profile"]["info_cache"].as_object() {
        for (dir, info) in cache {
            if let Some(n) = info["name"].as_str().filter(|n| !n.is_empty()) {
                out.insert(dir.clone(), n.to_string());
            }
        }
    }
    out
}

fn is_profile_dir(n: &str) -> bool {
    n == "Default" || n == "Guest Profile" || n == "System Profile" || n.starts_with("Profile ")
}

fn browser(c: &Ctx, b: &Browser) -> Vec<StorageGroup> {
    let roots: Vec<PathBuf> = b.roots.iter().map(|r| c.home(r)).collect();
    let names = chromium_profile_names(&roots[0]);
    let icon = format!("app:{}", s(c.app));
    let running = c.app_running(c.app).then(|| format!("{} is open; quit it first", c.name));
    // profile dir → cache paths across roots
    let mut profiles: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut shared = vec![];
    for root in &roots {
        for p in util::subdirs(root) {
            let n = name_of(&p);
            if is_profile_dir(&n) {
                let e = profiles.entry(n).or_default();
                for cache in CHROMIUM_CACHES {
                    e.extend(expand(&p, cache));
                }
            }
        }
        for cache in CHROMIUM_SHARED {
            shared.extend(expand(root, cache));
        }
    }
    let mut items: Vec<Finding> = profiles
        .into_iter()
        .filter(|(_, paths)| !paths.is_empty())
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(dir, paths)| {
            let u = size::usage_all(&strs(&paths));
            let title = names.get(&dir).cloned().unwrap_or_else(|| dir.clone());
            item("cache.browser", Category::Cache, title, icon.clone(), strs(&paths), u, Safety::Safe, true, "Web pages, scripts and shaders this profile cached. Your history, logins and tabs stay.")
                .detail(if names.contains_key(&dir) { dir } else { String::new() })
                .busy(running.clone())
        })
        .collect();
    if !shared.is_empty() {
        let u = size::usage_all(&strs(&shared));
        items.push(item("cache.browser", Category::Cache, "Shared GPU caches", icon.clone(), strs(&shared), u, Safety::Safe, true, "Compiled shaders all profiles share; rebuilt on the next launch.").busy(running.clone()));
    }
    let mut groups = vec![group("profiles", "Cache per profile", "cache", None, items)];
    groups.push(group("caches", "App caches", "cache", None, library_caches(c, &icon, running)));
    groups
}

/// `~/Library/Caches/<bundle id>` and the updater's download folder.
fn library_caches(c: &Ctx, icon: &str, busy: Option<String>) -> Vec<Finding> {
    let mut rows: Vec<(PathBuf, &str, &str)> = vec![];
    let caches = c.home("Library/Caches");
    for p in util::children(&caches) {
        let n = name_of(&p);
        if !real_dir(&p) {
            continue;
        }
        if n.eq_ignore_ascii_case(c.id) {
            rows.push((p, "App cache", "Files the app cached; it fetches or makes them again."));
        } else if n.eq_ignore_ascii_case(&format!("{}.ShipIt", c.id)) || n.eq_ignore_ascii_case(&format!("{}-updater", c.name.to_lowercase())) {
            rows.push((p, "Update downloads", "Updates the app already downloaded and installed."));
        }
    }
    rows.into_par_iter()
        .map(|(p, title, why)| {
            let u = size::usage(&p);
            item("cache.app", Category::Cache, title, icon.to_string(), vec![s(&p)], u, Safety::Safe, true, why).busy(busy.clone())
        })
        .filter(|f| f.bytes > 0)
        .collect()
}

/// The app's folder in Application Support, by app name, bundle name or bundle id.
fn electron_support(c: &Ctx) -> Option<PathBuf> {
    let support = c.home("Library/Application Support");
    let wanted: Vec<String> = [Some(c.name), c.bundle_name, Some(c.id)].into_iter().flatten().map(|n| n.to_lowercase()).collect();
    let kids = util::subdirs(&support);
    for w in &wanted {
        if let Some(p) = kids.iter().find(|p| name_of(p).to_lowercase() == *w && real_dir(p)) {
            return Some(p.clone());
        }
    }
    None
}

fn electron(c: &Ctx) -> Vec<StorageGroup> {
    let icon = format!("app:{}", s(c.app));
    let running = c.app_running(c.app).then(|| format!("{} is open; quit it first", c.name));
    let mut items: Vec<Finding> = vec![];
    if let Some(support) = electron_support(c) {
        items = ELECTRON_ROWS
            .par_iter()
            .filter_map(|(title, rels, why)| {
                let paths: Vec<PathBuf> = rels.iter().flat_map(|r| expand(&support, r)).collect();
                if paths.is_empty() {
                    return None;
                }
                let u = size::usage_all(&strs(&paths));
                Some(item("cache.electron", Category::Cache, *title, icon.clone(), strs(&paths), u, Safety::Safe, true, *why).busy(running.clone()))
            })
            .filter(|f| f.bytes > 0)
            .collect();
    }
    items.extend(library_caches(c, &icon, running));
    vec![group("caches", "Caches", "cache", Some("The app makes these again when it needs them. Your chats, settings and sign-ins stay."), items)]
}

// ---------- coding-agent CLIs ----------

/// Why this version must stay, if it must: in use by the launcher, the newest, or not a plain
/// version entry.
fn cli_keep(path: &Path, root: &Path, launcher: &Path) -> Option<String> {
    if path.parent() != Some(root) || parse_version(&name_of(path)).is_none() {
        return Some("Not a version Disko recognises".into());
    }
    let current = fs::canonicalize(launcher).ok().filter(|p| p.starts_with(root));
    let Some(current) = current else { return Some("Couldn't tell which version is in use".into()) };
    if current.starts_with(path) {
        return Some("In use".into());
    }
    let newest = util::children(root).iter().filter_map(|p| parse_version(&name_of(p))).max();
    if parse_version(&name_of(path)) == newest {
        return Some("Newest; kept for updates".into());
    }
    None
}

fn cli_versions(c: &Ctx, cli: &Cli) -> Vec<StorageGroup> {
    let root = c.home(cli.versions);
    let launcher = c.home(cli.launcher);
    let items: Vec<Finding> = util::children(&root)
        .into_iter()
        .filter(|p| cli_keep(p, &root, &launcher).is_none())
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|p| {
            let u = size::usage(&p);
            item("versions.agent-cli", Category::OldVersions, format!("{} {}", cli.title, name_of(&p)), "terminal", vec![s(&p)], u, Safety::Safe, false, "An older version; the one in use and the newest stay.")
                .used(size::own_mtime(&p))
        })
        .collect();
    vec![group("cli-versions", &format!("Older {} versions", cli.title), "terminal", None, items)]
}

// ---------- recheck before removing ----------

/// Right before removing: storage items that must stay after all, with why. Covers what the
/// generic busy check can't see (a simulator booted meanwhile, an Adobe app opened, a CLI
/// version that became the current one).
pub fn recheck(items: &[Finding]) -> Vec<(String, String)> {
    recheck_in(&StorageEnv::real(), items)
}

pub fn recheck_in(env: &StorageEnv, items: &[Finding]) -> Vec<(String, String)> {
    let mut out = vec![];
    let procs = if env.processes { util::process_paths() } else { vec![] };
    // Simulators and runtimes: ask simctl once.
    let sims: Vec<&Finding> = items.iter().filter(|f| f.rule == "developer.simulator").collect();
    let rts: Vec<&Finding> = items.iter().filter(|f| f.rule == "extras.simulator-runtime").collect();
    let last_arg = |f: &Finding| match &f.action {
        Action::Command { argv, .. } => argv.last().cloned().unwrap_or_default(),
        _ => String::new(),
    };
    if !sims.is_empty() || !rts.is_empty() {
        let dev = sims.iter().chain(rts.iter()).find_map(|f| match &f.action {
            Action::Command { argv, .. } => argv.iter().find_map(|a| a.strip_prefix("DEVELOPER_DIR=").map(str::to_string)),
            _ => None,
        });
        let (devices, runtimes) = match &env.simctl {
            Some((d, r)) => (Some(d.clone()), Some(r.clone())),
            None if env.tools => rayon::join(
                || simctl_run(dev.as_deref(), &["list", "devices", "-j"]),
                || if rts.is_empty() { None } else { simctl_run(dev.as_deref(), &["runtime", "list", "-j"]) },
            ),
            None => (None, None),
        };
        match devices.as_deref().and_then(parse_devices) {
            None => out.extend(sims.iter().chain(rts.iter()).map(|f| (f.id.clone(), "Couldn't ask the Simulator about it".to_string()))),
            Some(list) => {
                for f in &sims {
                    if list.iter().any(|d| d.udid == last_arg(f) && d.state != "Shutdown") {
                        out.push((f.id.clone(), "Running now; shut it down first".into()));
                    }
                }
                let runtimes = runtimes.as_deref().and_then(parse_runtimes).unwrap_or_default();
                for f in &rts {
                    let Some(r) = runtimes.iter().find(|r| r.identifier == last_arg(f)) else { continue };
                    if let Some(d) = list.iter().find(|d| d.runtime == r.runtime_id && d.state != "Shutdown") {
                        out.push((f.id.clone(), format!("{} is running on it", d.name)));
                    }
                }
            }
        }
    }
    if items.iter().any(|f| f.rule.starts_with("cache.adobe")) {
        if let Some(why) = adobe_running(&adobe_apps(&env.apps_dir), &procs) {
            out.extend(items.iter().filter(|f| f.rule.starts_with("cache.adobe")).map(|f| (f.id.clone(), why.clone())));
        }
    }
    for f in items.iter().filter(|f| f.rule == "versions.agent-cli") {
        for p in &f.paths {
            let path = Path::new(p);
            if let Some(cli) = CLIS.iter().find(|c| path.parent() == Some(env.home.join(c.versions).as_path())) {
                if let Some(why) = cli_keep(path, &env.home.join(cli.versions), &env.home.join(cli.launcher)) {
                    out.push((f.id.clone(), why));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::make_app;
    use crate::util::test_dir;

    fn touch(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![1u8; bytes]).unwrap();
    }

    fn set_id(app: &Path, id: &str, version: &str) {
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        d.insert("CFBundleShortVersionString".into(), version.into());
        plist::Value::Dictionary(d).to_file_xml(app.join("Contents/Info.plist")).unwrap();
    }

    const DEVICES: &str = r#"{
      "devices" : {
        "com.apple.CoreSimulator.SimRuntime.iOS-27-0" : [
          { "dataPath" : "/x/data", "dataPathSize" : 5125910528, "udid" : "15A8FBBE-9615-4880-8443-D291C0237788",
            "lastUsedAt" : "2026-11-15T15:42:19Z", "isAvailable" : true,
            "deviceTypeIdentifier" : "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro", "state" : "Shutdown", "name" : "iPhone 17 Pro" },
          { "udid" : "25A8FBBE-9615-4880-8443-D291C0237788", "isAvailable" : true, "dataPathSize" : 1000,
            "deviceTypeIdentifier" : "com.apple.CoreSimulator.SimDeviceType.iPad-Air-11-inch-M3", "state" : "Booted", "name" : "iPad Air" }
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-10-2" : [
          { "udid" : "35A8FBBE-9615-4880-8443-D291C0237788", "isAvailable" : false,
            "deviceTypeIdentifier" : "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-9-45mm", "state" : "Shutdown", "name" : "Apple Watch" }
        ]
      }
    }"#;

    const RUNTIMES: &str = r#"{
      "56857D2B-D7CD-4468-92FC-2D140F3552C3" : {
        "build" : "24A434", "deletable" : true, "identifier" : "56857D2B-D7CD-4468-92FC-2D140F3552C3",
        "lastUsedAt" : "2026-11-06T18:58:47Z", "platformIdentifier" : "com.apple.platform.iphonesimulator",
        "runtimeIdentifier" : "com.apple.CoreSimulator.SimRuntime.iOS-27-0", "sizeBytes" : 8067000161, "state" : "Ready", "version" : "27.0"
      },
      "66857D2B-D7CD-4468-92FC-2D140F3552C3" : {
        "build" : "22A100", "deletable" : true, "identifier" : "66857D2B-D7CD-4468-92FC-2D140F3552C3",
        "platformIdentifier" : "com.apple.platform.appletvsimulator",
        "runtimeIdentifier" : "com.apple.CoreSimulator.SimRuntime.tvOS-18-0", "sizeBytes" : 4000000000, "state" : "Ready", "version" : "18.0"
      }
    }"#;

    #[test]
    fn parses_simctl() {
        let sims = parse_devices(DEVICES).unwrap();
        assert_eq!(sims.len(), 3);
        let p = sims.iter().find(|d| d.name == "iPhone 17 Pro").unwrap();
        assert_eq!(p.data_size, Some(5125910528));
        assert_eq!(p.last_used, util::parse_time("2026-11-15T15:42:19Z"));
        assert!(!sims.iter().find(|d| d.name == "Apple Watch").unwrap().available);
        let rts = parse_runtimes(RUNTIMES).unwrap();
        assert_eq!(rts.len(), 2);
        assert_eq!(rts[0].platform, "iOS");
        assert_eq!(rts[1].platform, "tvOS");
        assert_eq!(runtime_label("com.apple.CoreSimulator.SimRuntime.iOS-27-0"), "iOS 27.0");
        assert_eq!(runtime_label("com.apple.CoreSimulator.SimRuntime.xrOS-2-0"), "visionOS 2.0");
        assert!(parse_devices("not json").is_none());
    }

    #[test]
    fn xcode_profile_lists_everything() {
        let home = test_dir("storage-xcode");
        let apps = home.join("Applications");
        let xcode = make_app(&apps, "Xcode", "com.apple.dt.Xcode");
        let old = make_app(&apps, "Xcode-16.4", "com.apple.dt.Xcode");
        set_id(&old, "com.apple.dt.Xcode", "16.4");
        let dev = home.join("Library/Developer");
        touch(&dev.join("CoreSimulator/Devices/15A8FBBE-9615-4880-8443-D291C0237788/device.plist"), 100);
        touch(&dev.join("Xcode/iOS DeviceSupport/iPhone16,2 18.4 (22E240)/Symbols/x"), 50_000);
        touch(&dev.join("Xcode/iOS DeviceSupport/17.4 (21E219)/Symbols/x"), 30_000);
        touch(&dev.join("Xcode/watchOS DeviceSupport/Watch7,1 10.2 (21S364)/Symbols/x"), 30_000);
        touch(&dev.join("Xcode/DerivedData/Disko-abcdefghijklmnopqrstuvwxyz/Build/x"), 70_000);
        touch(&dev.join("Xcode/DerivedData/ModuleCache.noindex/x"), 10_000);
        let mut info = plist::Dictionary::new();
        info.insert("Name".into(), "Disko".into());
        let mut props = plist::Dictionary::new();
        props.insert("CFBundleShortVersionString".into(), "1.0".into());
        props.insert("CFBundleVersion".into(), "42".into());
        info.insert("ApplicationProperties".into(), plist::Value::Dictionary(props));
        let arch = dev.join("Xcode/Archives/2026-11-01/Disko 01.11.26, 10.00.xcarchive");
        touch(&arch.join("Products/x"), 40_000);
        plist::Value::Dictionary(info).to_file_xml(arch.join("Info.plist")).unwrap();
        touch(&dev.join("CoreSimulator/Caches/dyld/x"), 20_000);
        touch(&home.join("Library/Caches/com.apple.dt.Xcode/x"), 20_000);

        let mut env = StorageEnv::fixture(&home);
        env.simctl = Some((DEVICES.into(), RUNTIMES.into()));
        let st = storage_in(&env, &xcode).unwrap();
        assert_eq!(st.app.profile, "xcode");
        let g = |id: &str| st.groups.iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no group {id}: {:?}", st.groups.iter().map(|g| &g.id).collect::<Vec<_>>()));

        // Every simulator, one by one; the running one is busy and listed first.
        let sims = g("simulators");
        assert_eq!(sims.items.len(), 3);
        assert_eq!(sims.items[0].title, "iPad Air");
        assert!(sims.items[0].busy.is_some());
        let phone = sims.items.iter().find(|f| f.title == "iPhone 17 Pro").unwrap();
        assert!(phone.bytes >= 5125910528);
        assert_eq!(phone.detail.as_deref(), Some("iOS 27.0"));
        assert_eq!(phone.paths, vec![s(&dev.join("CoreSimulator/Devices/15A8FBBE-9615-4880-8443-D291C0237788"))]);
        assert_eq!(phone.safety, Safety::Review);
        match &phone.action {
            Action::Command { argv, .. } => assert_eq!(argv, &simctl_argv(None, &["delete", "15A8FBBE-9615-4880-8443-D291C0237788"])),
            a => panic!("{a:?}"),
        }
        assert!(sims.items.iter().find(|f| f.title == "Apple Watch").unwrap().detail.as_deref().unwrap().contains("not installed"));

        // Every runtime, with how many simulators use it; same id as the clutter finding.
        let rts = g("runtimes");
        assert_eq!(rts.items.len(), 2);
        let ios = rts.items.iter().find(|f| f.title == "iOS 27.0").unwrap();
        assert_eq!(ios.bytes, 8067000161);
        assert_eq!(ios.detail.as_deref(), Some("Used by 2 simulators"));
        assert!(ios.busy.as_deref().unwrap().contains("iPad Air"));
        assert_eq!(ios.id, finding_id("extras.simulator-runtime", &["runtime:56857D2B-D7CD-4468-92FC-2D140F3552C3".into()]));
        let tv = rts.items.iter().find(|f| f.title == "tvOS 18.0").unwrap();
        assert_eq!(tv.detail.as_deref(), Some("Used by no simulator"));
        assert!(tv.busy.is_none() && tv.paths.is_empty());

        let ds = g("device-support");
        assert_eq!(ds.items.len(), 3);
        assert!(ds.items.iter().any(|f| f.title == "iOS 18.4 (22E240)" && f.detail.as_deref() == Some("iPhone16,2") && f.icon == "iphone"));
        assert!(ds.items.iter().any(|f| f.title == "watchOS 10.2 (21S364)" && f.icon == "watch"));
        assert!(ds.items.iter().all(|f| f.safety == Safety::Safe && f.action == Action::Delete));

        let dd = g("derived-data");
        assert!(dd.items.iter().any(|f| f.title == "Disko"));
        assert!(dd.items.iter().any(|f| f.title == "Module cache"));

        let ar = g("archives");
        assert_eq!(ar.items.len(), 1);
        assert_eq!(ar.items[0].title, "Disko");
        assert_eq!(ar.items[0].detail.as_deref(), Some("1.0 (42)"));
        assert_eq!(ar.items[0].action, Action::Trash);

        let caches = g("caches");
        assert!(caches.items.iter().any(|f| f.title == "Simulator caches"));
        assert!(caches.items.iter().any(|f| f.title == "Xcode cache"));

        let xs = g("xcodes");
        assert_eq!(xs.items.len(), 1);
        assert_eq!(xs.items[0].paths, vec![s(&old)]);
        assert_eq!(xs.items[0].detail.as_deref(), Some("Version 16.4"));

        assert_eq!(st.bytes, st.groups.iter().map(|g| g.bytes).sum::<u64>());
        // Ids are unique across the page.
        let mut ids: Vec<&String> = st.groups.iter().flat_map(|g| g.items.iter().map(|f| &f.id)).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);

        // Without simctl answering, simulators are listed but none can be removed.
        let mut env2 = StorageEnv::fixture(&home);
        env2.simctl = Some(("broken".into(), "broken".into()));
        let st2 = storage_in(&env2, &xcode).unwrap();
        let sims2 = st2.groups.iter().find(|g| g.id == "simulators").unwrap();
        assert_eq!(sims2.items.len(), 1);
        assert!(sims2.items[0].busy.is_some());
        assert!(!st2.groups.iter().any(|g| g.id == "runtimes"));

        // Recheck: a simulator that booted meanwhile stays.
        let booted = DEVICES.replace("\"state\" : \"Shutdown\", \"name\" : \"iPhone 17 Pro\"", "\"state\" : \"Booted\", \"name\" : \"iPhone 17 Pro\"");
        let mut env3 = StorageEnv::fixture(&home);
        env3.simctl = Some((booted, RUNTIMES.into()));
        let blocked = recheck_in(&env3, &[phone.clone()]);
        assert_eq!(blocked.len(), 1);
        assert!(recheck_in(&env, &[phone.clone()]).is_empty());
        // …and so does the runtime under a running simulator.
        assert_eq!(recheck_in(&env, &[ios.clone(), tv.clone()]), vec![(ios.id.clone(), "iPad Air is running on it".to_string())]);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn simctl_through_developer_dir() {
        let argv = simctl_argv(Some("/Applications/Xcode.app/Contents/Developer"), &["delete", "X"]);
        assert_eq!(argv, vec!["/usr/bin/env", "DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer", "/usr/bin/xcrun", "simctl", "delete", "X"]);
        assert_eq!(simctl_argv(None, &["runtime", "delete", "Y"]), vec!["/usr/bin/xcrun", "simctl", "runtime", "delete", "Y"]);
    }

    #[test]
    fn adobe_profile_offers_caches_and_old_versions_only() {
        let home = test_dir("storage-adobe");
        let apps = home.join("Applications");
        let ae = make_app(&apps.join("Adobe After Effects 2026"), "Adobe After Effects 2026", "com.adobe.AfterEffects.application");
        set_id(&ae, "com.adobe.AfterEffects.application", "26.5.2");
        let pr = make_app(&apps.join("Adobe Premiere Pro 2026"), "Adobe Premiere Pro 2026", "com.adobe.PremierePro.26");
        set_id(&pr, "com.adobe.PremierePro.26", "26.5.2");
        let ps = make_app(&apps.join("Adobe Photoshop 2026"), "Adobe Photoshop 2026", "com.adobe.Photoshop");
        set_id(&ps, "com.adobe.Photoshop", "27.1.0");
        let lib = home.join("Library");
        for p in [
            "Application Support/Adobe/Common/Media Cache Files/a.cfa",
            "Application Support/Adobe/Common/Media Cache/db",
            "Application Support/Adobe/Common/Peak Files/a.pek",
            "Application Support/Adobe/Common/Motion Graphics Templates/t.mogrt",
            "Application Support/Adobe/Common/LUTs/l.cube",
            "Application Support/Adobe/Adobe Photoshop 2024/x",
            "Application Support/Adobe/Adobe Photoshop 2026/x",
            "Preferences/Adobe/After Effects/24.6/prefs.txt",
            "Preferences/Adobe/After Effects/26.0/Debug Database.txt",
            "Preferences/Adobe/After Effects/26.5/prefs.txt",
            "Caches/Adobe/After Effects/26.5/Disk Cache - mac/f",
            "Caches/Adobe/After Effects/26.5/Presets/p",
            "Preferences/Adobe Photoshop 2024 Settings/s",
            "Preferences/com.unrelated 2024/s",
        ] {
            touch(&lib.join(p), 20_000);
        }
        // A media cache moved elsewhere in Premiere's preferences.
        let custom = home.join("Scratch/Media Cache Files");
        touch(&custom.join("b.cfa"), 20_000);
        let prefs = home.join("Documents/Adobe/Premiere Pro/26.0/Profile-me/Adobe Premiere Pro Prefs");
        touch(&prefs, 0);
        fs::write(&prefs, format!("<x><BE.Prefs.MediaCache.FilesSideBySide>false</BE.Prefs.MediaCache.FilesSideBySide><BE.Prefs.MediaCache.FolderPath>{}</BE.Prefs.MediaCache.FolderPath></x>", s(&custom))).unwrap();

        let env = StorageEnv::fixture(&home);
        let st = storage_in(&env, &pr).unwrap();
        assert_eq!(st.app.profile, "adobe");
        let all: Vec<&Finding> = st.groups.iter().flat_map(|g| g.items.iter()).collect();
        let paths: Vec<String> = all.iter().flat_map(|f| f.paths.iter().cloned()).collect();
        let has = |rel: &str| paths.iter().any(|p| p.ends_with(rel));
        assert!(has("Common/Media Cache Files") && has("Common/Media Cache") && has("Common/Peak Files"));
        assert!(has("Disk Cache - mac"));
        assert!(paths.contains(&s(&custom)), "{paths:?}");
        assert!(has("Adobe/Adobe Photoshop 2024") && has("Adobe Photoshop 2024 Settings") && has("After Effects/24.6"));
        // Never: templates, LUTs, presets, the installed versions, other vendors.
        for never in ["Motion Graphics Templates", "LUTs", "Presets", "Adobe Photoshop 2026", "After Effects/26.0", "After Effects/26.5", "com.unrelated"] {
            assert!(!paths.iter().any(|p| p.contains(never) && !p.contains("Disk Cache")), "{never} offered: {paths:?}");
        }
        let ps_old = all.iter().find(|f| f.title == "Adobe Photoshop 2024").unwrap();
        assert_eq!(ps_old.paths.len(), 2);
        assert_eq!(ps_old.action, Action::Trash);
        assert!(all.iter().filter(|f| f.rule.starts_with("cache.")).all(|f| f.safety == Safety::Safe && f.regrows));
        // Creative Cloud itself has no page.
        let cc = make_app(&apps.join("Adobe Creative Cloud"), "Adobe Creative Cloud", "com.adobe.acc.AdobeCreativeCloud");
        assert!(storage_in(&env, &cc).is_none());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn browsers_and_electron_apps() {
        let home = test_dir("storage-web");
        let apps = home.join("Applications");
        let chrome = make_app(&apps, "Google Chrome", "com.google.Chrome");
        let lib = home.join("Library");
        for p in [
            "Application Support/Google/Chrome/Default/Code Cache/js/x",
            "Application Support/Google/Chrome/Default/GPUCache/x",
            "Application Support/Google/Chrome/Default/History",
            "Application Support/Google/Chrome/Default/Cookies",
            "Application Support/Google/Chrome/Profile 1/GPUCache/x",
            "Application Support/Google/Chrome/GrShaderCache/x",
            "Caches/Google/Chrome/Default/Cache/Cache_Data/x",
            "Caches/com.google.Chrome/x",
        ] {
            touch(&lib.join(p), 20_000);
        }
        fs::write(lib.join("Application Support/Google/Chrome/Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Levi"},"Profile 1":{"name":"Work"}}}}"#).unwrap();
        let env = StorageEnv::fixture(&home);
        let st = storage_in(&env, &chrome).unwrap();
        assert_eq!(st.app.profile, "browser");
        let prof = st.groups.iter().find(|g| g.id == "profiles").unwrap();
        let levi = prof.items.iter().find(|f| f.title == "Levi").unwrap();
        assert_eq!(levi.paths.len(), 3);
        assert!(prof.items.iter().any(|f| f.title == "Work"));
        assert!(prof.items.iter().any(|f| f.title == "Shared GPU caches"));
        let paths: Vec<&String> = st.groups.iter().flat_map(|g| g.items.iter().flat_map(|f| f.paths.iter())).collect();
        assert!(!paths.iter().any(|p| p.ends_with("History") || p.ends_with("Cookies")));
        assert!(st.groups.iter().any(|g| g.id == "caches" && g.items.iter().any(|f| f.title == "App cache")));

        // An Electron app with a coding-agent CLI beside it.
        let claude = make_app(&apps, "Claude", "com.anthropic.claudefordesktop");
        fs::create_dir_all(claude.join("Contents/Frameworks/Electron Framework.framework")).unwrap();
        for p in ["Application Support/Claude/Cache/Cache_Data/x", "Application Support/Claude/Code Cache/js/x", "Application Support/Claude/Partitions/p1/Cache/x", "Application Support/Claude/config.json", "Caches/com.anthropic.claudefordesktop.ShipIt/update.zip"] {
            touch(&lib.join(p), 20_000);
        }
        let versions = home.join(".local/share/claude/versions");
        for v in ["2.1.9", "2.1.10", "2.1.11", "2.1.12"] {
            touch(&versions.join(v), 30_000);
        }
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::os::unix::fs::symlink(versions.join("2.1.10"), home.join(".local/bin/claude")).unwrap();
        let st = storage_in(&env, &claude).unwrap();
        assert_eq!(st.app.profile, "electron");
        let caches = st.groups.iter().find(|g| g.id == "caches").unwrap();
        let web = caches.items.iter().find(|f| f.title == "Web cache").unwrap();
        assert_eq!(web.paths.len(), 2);
        assert!(caches.items.iter().any(|f| f.title == "Update downloads"));
        assert!(!caches.items.iter().any(|f| f.paths.iter().any(|p| p.ends_with("config.json"))));
        let cli = st.groups.iter().find(|g| g.id == "cli-versions").unwrap();
        let mut titles: Vec<&str> = cli.items.iter().map(|f| f.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, vec!["Claude Code 2.1.11", "Claude Code 2.1.9"]);
        // The launcher moved to 2.1.11 meanwhile: that one stays.
        fs::remove_file(home.join(".local/bin/claude")).unwrap();
        std::os::unix::fs::symlink(versions.join("2.1.11"), home.join(".local/bin/claude")).unwrap();
        let v11 = cli.items.iter().find(|f| f.title.ends_with("2.1.11")).unwrap();
        let v9 = cli.items.iter().find(|f| f.title.ends_with("2.1.9")).unwrap();
        let blocked = recheck_in(&env, &[v11.clone(), v9.clone()]);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].0, v11.id);

        // Apps without a profile have no page.
        let plain = make_app(&apps, "Plain", "com.acme.Plain");
        assert!(storage_in(&env, &plain).is_none());
        fs::remove_dir_all(&home).unwrap();
    }

    /// Read-only look at this Mac: `cargo test -p disko-apps live_storage -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real Mac"]
    fn live_storage() {
        let apps = std::env::var("DISKO_LIVE_APPS").unwrap_or_else(|_| "/Applications/Xcode.app".into());
        for app in apps.split(':') {
            let t = Instant::now();
            let Some(st) = storage(Path::new(app)) else { println!("{app}: no page"); continue };
            println!("{} ({}): {} bytes in {:?}", st.app.name, st.app.profile, st.bytes, t.elapsed());
            for g in &st.groups {
                println!("  {} · {} items · {} bytes", g.title, g.items.len(), g.bytes);
                for f in g.items.iter().take(6) {
                    println!("    {} | {} | {} | {:?} | {:?}", f.title, f.detail.as_deref().unwrap_or(""), f.bytes, f.busy, f.action);
                }
            }
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(product_key("Adobe After Effects 2026"), ("after effects".into(), Some(2026)));
        assert_eq!(product_key("Adobe Photoshop 2024 Settings"), ("photoshop".into(), Some(2024)));
        assert_eq!(product_key("Premiere Pro"), ("premiere pro".into(), None));
        assert_eq!(product_key("Adobe Media Encoder"), ("media encoder".into(), None));
        assert_eq!(support_name("iPhone15,2 17.4 (21E219)"), ("iPhone15,2".into(), "17.4 (21E219)".into()));
        assert_eq!(support_name("17.4 (21E219)"), ("".into(), "17.4 (21E219)".into()));
        assert_eq!(derived_name("Disko-abcdefghijklmnopqrstuvwxyz"), "Disko");
        let pairs = ae_pairs("[\"Disk Cache\"]\t\"Disk Cache Folder\" = \"/Volumes/Fast/AE\"\t\"Other\" = \"1\"");
        assert!(pairs.contains(&("Disk Cache Folder".into(), "/Volumes/Fast/AE".into())));
        assert_eq!(expand(Path::new("/nonexistent"), "Partitions/*/Cache"), Vec::<PathBuf>::new());
    }
}
