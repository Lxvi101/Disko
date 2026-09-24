//! Live developer-storage inventory, independent of the disk scan.
use serde::Serialize;
use serde_json::Value;
use std::{collections::{HashMap, HashSet}, fs, os::unix::fs::MetadataExt, path::{Path, PathBuf}, process::Command, time::UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct XcodeItem {
    pub path: String, pub name: String, pub group: String, pub total: u64,
    pub modified: Option<u64>, pub last_booted: Option<String>, pub subtitle: String,
    pub badges: Vec<String>, pub blocked: Option<String>, pub partial: bool,
}
#[derive(Debug, Serialize)]
pub struct Inventory { pub items: Vec<XcodeItem>, pub warnings: Vec<String> }

pub fn simulator_devices(home: &Path) -> Result<Value, String> {
    let out = Command::new("/usr/bin/xcrun").args(["simctl", "--set"])
        .arg(home.join("Library/Developer/CoreSimulator/Devices"))
        .args(["list", "devices", "--json"]).output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err("Simulator status is unavailable. Open Xcode, check its selected command-line tools, then refresh. Device cleanup is disabled until status can be verified.".into()); }
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    if !json["devices"].is_object() { return Err("Simulator returned an invalid device list".into()); }
    Ok(json)
}
fn device_map(json: &Value) -> HashMap<String, Value> {
    json["devices"].as_object().into_iter().flat_map(|o| o.values())
        .filter_map(Value::as_array).flatten().filter_map(|d| Some((d["udid"].as_str()?.into(), d.clone()))).collect()
}
pub fn simulator_id(path: &Path, home: &Path) -> Result<Option<String>, String> {
    let root = home.join("Library/Developer/CoreSimulator/Devices");
    if !path.starts_with(&root) { return Ok(None); }
    let rel = path.strip_prefix(&root).unwrap();
    let id = rel.to_str().unwrap_or("");
    if rel.components().count() != 1 || !valid_uuid(id) {
        return Err("Select a whole simulator device on the Xcode page, not its internal files or device registry.".into());
    }
    Ok(Some(id.into()))
}
fn valid_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i,b)| if [8,13,18,23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}
pub fn check_simulator(path: &Path, home: &Path) -> Result<Option<String>, String> {
    let Some(id) = simulator_id(path, home)? else { return Ok(None) };
    let devices = device_map(&simulator_devices(home)?);
    let device = devices.get(&id).ok_or("Device is not registered with CoreSimulator; inspect it in Xcode before cleanup")?;
    if device["state"].as_str() != Some("Shutdown") { return Err("Shut down this simulator in Xcode or Simulator before cleaning it up".into()); }
    Ok(Some(id))
}
pub fn delete_simulator(id: &str, home: &Path) -> Result<(), String> {
    if !valid_uuid(id) { return Err("Invalid simulator identifier".into()); }
    let out = Command::new("/usr/bin/xcrun").args(["simctl", "--set"])
        .arg(home.join("Library/Developer/CoreSimulator/Devices"))
        .args(["delete", id]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().into()) }
}

pub fn allocated_size(path: &Path) -> (u64, bool) {
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut partial = false;
    for entry in walkdir::WalkDir::new(path).follow_links(false).same_file_system(true) {
        match entry.and_then(|e| e.metadata()) {
            Ok(m) => { if seen.insert((m.dev(), m.ino())) { total = total.saturating_add(m.blocks().saturating_mul(512)); } }
            Err(_) => partial = true,
        }
    }
    (total, partial)
}
fn plist_string(p: &plist::Value, key: &str) -> String {
    p.as_dictionary().and_then(|d| d.get(key)).and_then(plist::Value::as_string).unwrap_or("").into()
}
fn runtime_label(id: &str) -> String {
    let raw = id.rsplit('.').next().unwrap_or(id);
    let (platform, version) = raw.split_once('-').unwrap_or((raw, ""));
    format!("{} {}", platform, version.replace('-', ".")).trim().into()
}
fn support_version(name: &str) -> (String, Vec<u32>) {
    let mut parts = name.split_whitespace();
    let first = parts.next().unwrap_or("");
    let (model, version) = if first.starts_with(|c: char| c.is_ascii_digit()) { ("Device", first) } else { (first, parts.next().unwrap_or("")) };
    (model.into(), version.split('.').map(|n| n.parse().unwrap_or(0)).collect())
}
pub fn inventory(home: &Path) -> Inventory {
    inventory_with_devices(home, simulator_devices(home))
}

fn inventory_with_devices(home: &Path, snapshot: Result<Value, String>) -> Inventory {
    let mut report = Inventory { items: vec![], warnings: vec![] };
    let devices = match snapshot { Ok(v) => Some(device_map(&v)), Err(e) => { report.warnings.push(e); None } };
    let roots = [
        ("simulators", "Library/Developer/CoreSimulator/Devices"),
        ("support", "Library/Developer/Xcode/iOS DeviceSupport"),
        ("support", "Library/Developer/Xcode/watchOS DeviceSupport"),
        ("support", "Library/Developer/Xcode/tvOS DeviceSupport"),
        ("support", "Library/Developer/Xcode/visionOS DeviceSupport"),
        ("derived", "Library/Developer/Xcode/DerivedData"),
        ("mcp", "Library/Developer/XcodeBuildMCP"),
        ("mcp", "Library/Developer/XcodeBuildMCP/workspaces"),
        ("caches", "Library/Caches/com.apple.dt.Xcode"),
        ("caches", "Library/Caches/com.apple.dt.xcodebuild"),
        ("caches", "Library/Developer/CoreSimulator/Caches"),
        ("archives", "Library/Developer/Xcode/Archives"),
    ];
    let mut sim_keys = HashMap::<String, String>::new();
    for (group, relative) in roots {
        let root = home.join(relative);
        let entries = match fs::read_dir(&root) {
            Ok(e) => e, Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => { report.warnings.push(format!("{}: {}", root.display(), e)); continue; }
        };
        for entry in entries {
            let entry = match entry { Ok(e) => e, Err(e) => { report.warnings.push(e.to_string()); continue; } };
            let path = entry.path();
            let raw_name = entry.file_name().to_string_lossy().to_string();
            if (group == "mcp" && relative.ends_with("XcodeBuildMCP") && raw_name == "workspaces") || raw_name.starts_with('.') || (group == "simulators" && !valid_uuid(&raw_name)) { continue; }
            let meta = match fs::symlink_metadata(&path) { Ok(m) if !m.file_type().is_symlink() => m, _ => continue };
            let (total, partial) = allocated_size(&path);
            let mut item = XcodeItem { path: path.to_string_lossy().into(), name: raw_name.clone(), group: group.into(), total,
                modified: meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|t| t.as_secs()),
                last_booted: None, subtitle: String::new(), badges: vec![], blocked: None, partial };
            if group == "simulators" {
                let p = plist::Value::from_file(path.join("device.plist")).unwrap_or(plist::Value::Dictionary(Default::default()));
                let name = plist_string(&p, "name");
                if !name.is_empty() { item.name = name; }
                let runtime = plist_string(&p, "runtime");
                item.subtitle = runtime_label(&runtime);
                sim_keys.insert(item.path.clone(), format!("{}:{}", plist_string(&p, "deviceType"), runtime));
                item.last_booted = p.as_dictionary().and_then(|d| d.get("lastBootedAt")).and_then(plist::Value::as_date).map(|d| d.to_xml_format());
                if let Some(device) = devices.as_ref().and_then(|d| d.get(&raw_name)) {
                    let state = device["state"].as_str().unwrap_or("Unknown");
                    item.badges.push(state.into());
                    if state != "Shutdown" { item.blocked = Some("Shut down this simulator before cleanup.".into()); }
                    if device["isAvailable"].as_bool() == Some(false) { item.badges.push("Runtime unavailable".into()); }
                } else { item.blocked = Some("Live device status could not be verified.".into()); }
            } else if group == "support" {
                let (model, _) = support_version(&raw_name);
                item.subtitle = if model.starts_with("iPad") { "Physical iPad · debug symbols" } else { "Physical device · debug symbols" }.into();
            } else if group == "derived" {
                let p = plist::Value::from_file(path.join("info.plist")).ok();
                item.subtitle = p.as_ref().map(|p| plist_string(p, "WorkspacePath")).unwrap_or_default();
                if let Some((name, hash)) = raw_name.rsplit_once('-') { if hash.len() >= 20 { item.name = name.into(); } }
                item.badges.push("Rebuildable".into());
            }
            report.items.push(item);
        }
    }
    let mut counts = HashMap::<String, usize>::new();
    for key in sim_keys.values() { *counts.entry(key.clone()).or_default() += 1; }
    let mut newest = HashMap::<(PathBuf, String), Vec<u32>>::new();
    for it in report.items.iter().filter(|i| i.group == "support") {
        let (model, ver) = support_version(&it.name);
        let key = (Path::new(&it.path).parent().unwrap().into(), model);
        let current = newest.entry(key).or_default(); if ver > *current { *current = ver; }
    }
    for it in &mut report.items {
        if sim_keys.get(&it.path).and_then(|k| counts.get(k)).copied().unwrap_or(0) > 1 { it.badges.push("Same model & OS".into()); }
        if it.group == "support" {
            let (model, ver) = support_version(&it.name);
            let key = (Path::new(&it.path).parent().unwrap().into(), model);
            if newest.get(&key).is_some_and(|v| ver < *v) { it.badges.push("Older version".into()); }
            else { it.badges.push("Newest stored".into()); }
        }
    }
    report.items.sort_by(|a,b| b.total.cmp(&a.total));
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_marks_old_support_and_lists_individual_workspaces() {
        let home = std::env::temp_dir().join(format!("disko-inventory-{}", std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let support = home.join("Library/Developer/Xcode/iOS DeviceSupport");
        for name in ["iPhone18,2 27.0 (24A435)", "iPhone18,2 26.5.2 (23F84)", "iPad16,6 26.6.2 (23G90)"] {
            fs::create_dir_all(support.join(name)).unwrap();
        }
        let workspaces = home.join("Library/Developer/XcodeBuildMCP/workspaces");
        fs::create_dir_all(workspaces.join("First-project")).unwrap();
        fs::create_dir_all(workspaces.join("Second-project")).unwrap();
        let devices = home.join("Library/Developer/CoreSimulator/Devices");
        let id = "173FF51E-87DE-4043-9570-36EA73A315A4";
        fs::create_dir_all(devices.join(id)).unwrap();
        fs::write(devices.join("device_set.plist"), "registry").unwrap();
        let report = inventory_with_devices(&home, Err("Unavailable".into()));
        assert_eq!(report.items.iter().filter(|i| i.group == "mcp").count(), 2);
        assert!(report.items.iter().find(|i| i.name.contains("26.5.2")).unwrap().badges.contains(&"Older version".into()));
        assert!(report.items.iter().find(|i| i.name.starts_with("iPad")).unwrap().badges.contains(&"Newest stored".into()));
        assert!(report.items.iter().find(|i| i.group == "simulators").unwrap().blocked.is_some());
        assert!(!report.items.iter().any(|i| i.name == "device_set.plist"));
        assert_eq!(report.warnings, vec!["Unavailable"]);
        fs::remove_dir_all(home).unwrap();
    }
    #[test] fn simulator_scope_is_exact() {
        let home = Path::new("/Users/test");
        let root = home.join("Library/Developer/CoreSimulator/Devices");
        let id = "173FF51E-87DE-4043-9570-36EA73A315A4";
        assert_eq!(simulator_id(&root.join(id), home).unwrap(), Some(id.into()));
        assert!(simulator_id(&root, home).is_err());
        assert!(simulator_id(&root.join(id).join("data"), home).is_err());
        assert!(simulator_id(&root.join("device_set.plist"), home).is_err());
    }
    #[test] fn versions_sort_numerically_per_model() {
        assert!(support_version("iPhone18,2 27.0 (24A435)").1 > support_version("iPhone18,2 26.5.2 (23F84)").1);
        assert!(support_version("iPad16,6 26.10").1 > support_version("iPad16,6 26.9").1);
        assert_eq!(runtime_label("com.apple.CoreSimulator.SimRuntime.iOS-26-3"), "iOS 26.3");
    }
}
