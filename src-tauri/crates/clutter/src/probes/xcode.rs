//! Xcode: build products, caches, archives, device symbols, simulators and runtimes.

use super::{fixed, MIN};
use crate::apps::read_app;
use crate::finding::{make, FindingExt};
use crate::procs::who;
use crate::util::{ago, child_dirs, du, du_all, name_of, now, parse_version, plist_dict, plist_str, run, s, shallow_newest, version_str, Size, DAY};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BUILD_TOOLS: &[&str] = &["xcodebuild", "XCBBuildService", "SWBBuildService", "swift-frontend", "clang", "ibtool", "actool"];

fn xcode_busy(c: &Clutter) -> Option<String> {
    c.procs().app_named("Xcode").map(|p| format!("{} is running", who(p)))
}

/// "MyApp-abcdefghijklmnopqrstuvwxyz" → "MyApp".
pub fn derived_name(dir: &str) -> String {
    match dir.rsplit_once('-') {
        Some((n, hash)) if hash.len() >= 20 && hash.chars().all(|c| c.is_ascii_lowercase()) => n.to_string(),
        _ => dir.to_string(),
    }
}

pub fn probe(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let dev = home.join("Library/Developer");
    let procs = c.procs();
    let building = procs.named(BUILD_TOOLS).map(|p| who(p));
    let mut out = vec![];

    // DerivedData: one finding per project.
    let dd = dev.join("Xcode/DerivedData");
    let derived: Vec<Finding> = child_dirs(&dd)
        .into_par_iter()
        .filter_map(|p| {
            let raw = name_of(&p);
            let size = du(&p);
            if size.bytes < MIN {
                return None;
            }
            if raw.ends_with(".noindex") {
                return Some(make("developer.derived-data", Category::Developer, "Xcode module cache", "xcode", vec![s(&p)], size, Safety::Safe, true, "Precompiled modules shared by all projects; rebuilt on the next build.").detail(raw).busy(building.clone().map(|b| format!("{} is building", b))));
            }
            let info = plist_dict(&p.join("info.plist"));
            let workspace = info.as_ref().and_then(|d| plist_str(d, "WorkspacePath"));
            let gone = workspace.as_ref().is_some_and(|w| !Path::new(w).exists());
            let title = workspace.as_ref().map(|w| name_of(Path::new(w)).trim_end_matches(".xcodeproj").trim_end_matches(".xcworkspace").to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| derived_name(&raw));
            let why = if gone {
                format!("Xcode build output for a project that no longer exists ({}).", workspace.as_deref().map(disko_core::paths::short).unwrap_or_default())
            } else {
                format!("Xcode build output and index; rebuilt on the next build. Last built {}.", ago(size.newest))
            };
            let hot = shallow_newest(&p).is_some_and(|t| now() - t < 3600);
            let busy = building.clone().filter(|_| hot).map(|b| format!("{} is building", b));
            Some(make("developer.derived-data", Category::Developer, title, "xcode", vec![s(&p)], size, Safety::Safe, true, why).detail("DerivedData").busy(busy))
        })
        .collect();
    out.extend(derived);

    for (rule, title, rels, why, cat) in [
        ("developer.xcode-caches", "Xcode caches", &["Library/Caches/com.apple.dt.Xcode", "Library/Caches/com.apple.dt.xcodebuild", "Library/Developer/Xcode/DocumentationCache", "Library/Developer/Xcode/Products"][..], "Caches Xcode refills as you work.", Category::Developer),
        ("developer.simulator-caches", "Simulator caches", &["Library/Developer/CoreSimulator/Caches"][..], "Caches the Simulator rebuilds when it boots a device.", Category::Developer),
        ("developer.test-devices", "Test simulator clones", &["Library/Developer/XCTestDevices"][..], "Simulator copies Xcode made for parallel test runs; made again on the next run.", Category::Developer),
        ("logs.device-logs", "iOS device logs", &["Library/Developer/Xcode/iOS Device Logs"][..], "Console logs Xcode copied off your devices.", Category::Logs),
        ("installers.xcode-downloads", "Xcode downloads", &["Library/Developer/DVTDownloads"][..], "Components Xcode downloaded to install them.", Category::Installers),
    ] {
        if let Some(f) = fixed(c, rule, cat, title, "xcode", rels, Safety::Safe, true, why, MIN) {
            out.push(f.busy(xcode_busy(c).or_else(|| building.clone().map(|b| format!("{} is building", b)))));
        }
    }

    out.extend(archives(c));
    out.extend(device_support(c));
    out.extend(old_xcodes(c));
    out
}

/// Archives: keep the newest per app; older ones are for symbolicating old releases.
fn archives(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join("Library/Developer/Xcode/Archives");
    let mut by_app: HashMap<String, Vec<(i64, PathBuf, String)>> = HashMap::new();
    for day in child_dirs(&root) {
        for a in child_dirs(&day) {
            if !name_of(&a).ends_with(".xcarchive") {
                continue;
            }
            let info = plist_dict(&a.join("Info.plist"));
            let name = info.as_ref().and_then(|d| plist_str(d, "Name")).unwrap_or_else(|| name_of(&a).split(' ').next().unwrap_or("").to_string());
            let version = info
                .as_ref()
                .and_then(|d| d.get("ApplicationProperties")?.as_dictionary().and_then(|p| plist_str(p, "CFBundleShortVersionString")))
                .unwrap_or_default();
            let created = info.as_ref().and_then(|d| d.get("CreationDate")?.as_date().map(|t| std::time::SystemTime::from(t))).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
            by_app.entry(name).or_default().push((created.or_else(|| crate::util::mtime(&a)).unwrap_or(0), a, version));
        }
    }
    let mut out = vec![];
    for (app, mut list) in by_app {
        if list.len() < 2 {
            continue;
        }
        list.sort_by_key(|x| x.0);
        let newest = list.pop().unwrap();
        let old: Vec<PathBuf> = list.iter().map(|x| x.1.clone()).collect();
        let size = du_all(&old);
        if size.bytes < MIN {
            continue;
        }
        let versions: Vec<String> = list.iter().map(|x| x.2.clone()).filter(|v| !v.is_empty()).collect();
        out.push(
            make("developer.archives", Category::Developer, app.clone(), "xcode", old.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("{} older archives; the newest ({}) stays. Keep any build whose crash reports you still need to symbolicate.", old.len(), if newest.2.is_empty() { ago(Some(newest.0)) } else { newest.2.clone() }))
                .keyed(&format!("archives:{}", app))
                .detail(if versions.is_empty() { "Xcode archives".into() } else { format!("Archives {}", versions.join(", ")) }),
        );
    }
    out
}

/// "iPhone15,2 17.4 (21E219)" → ("iPhone15,2", [17,4]); "17.4 (21E219)" → ("", [17,4]).
pub fn support_version(name: &str) -> (String, Vec<u64>) {
    let mut parts = name.split_whitespace();
    let first = parts.next().unwrap_or("");
    let (model, version) = if first.starts_with(|c: char| c.is_ascii_digit()) { ("", first) } else { (first, parts.next().unwrap_or("")) };
    (model.into(), parse_version(version).unwrap_or_default())
}

/// Device symbols: keep the newest per platform, the newest per device model, and anything Xcode
/// used in the last 90 days (the owner's devices). Older ones only.
fn device_support(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    for platform in ["iOS", "watchOS", "tvOS", "visionOS", "macOS"] {
        let root = c.env.home.join(format!("Library/Developer/Xcode/{} DeviceSupport", platform));
        let entries: Vec<(PathBuf, String, Vec<u64>, Option<i64>)> = child_dirs(&root)
            .into_iter()
            .map(|p| {
                let (m, v) = support_version(&name_of(&p));
                let used = shallow_newest(&p);
                (p, m, v, used)
            })
            .collect();
        let newest = entries.iter().map(|e| e.2.clone()).max().unwrap_or_default();
        let old: Vec<&(PathBuf, String, Vec<u64>, Option<i64>)> = entries
            .iter()
            .filter(|(_, m, v, used)| {
                let newest_for_model = !m.is_empty() && !entries.iter().any(|(_, m2, v2, _)| m2 == m && v2 > v);
                *v < newest && !newest_for_model && !used.is_some_and(|t| now() - t < 90 * DAY)
            })
            .collect();
        if old.is_empty() {
            continue;
        }
        let paths: Vec<PathBuf> = old.iter().map(|e| e.0.clone()).collect();
        let size = du_all(&paths);
        if size.bytes < MIN {
            continue;
        }
        let mut versions: Vec<String> = old.iter().map(|e| version_str(&e.2)).collect();
        versions.sort();
        versions.dedup();
        out.push(
            make("developer.device-support", Category::Developer, format!("{} device symbols", platform), "xcode", paths.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, format!("Debug symbols for {} {} — older than what your devices run ({} kept). Xcode copies them again if an old device connects.", platform, versions.join(", "), version_str(&newest)))
                .keyed(&s(&root))
                .detail(format!("{} old versions", paths.len()))
                .busy(xcode_busy(c)),
        );
    }
    out
}

/// Extra Xcode copies beside the selected/newest one.
fn old_xcodes(c: &Clutter) -> Vec<Finding> {
    let apps_dir = c.env.sys("/Applications");
    let xcodes: Vec<(PathBuf, Vec<u64>)> = crate::util::children(&apps_dir)
        .into_iter()
        .filter(|p| name_of(p).starts_with("Xcode") && name_of(p).ends_with(".app"))
        .filter_map(|p| {
            let v = read_app(&p)?.version.and_then(|v| parse_version(&v))?;
            Some((p, v))
        })
        .collect();
    if xcodes.len() < 2 {
        return vec![];
    }
    let selected = if c.env.tools { run(Path::new("/usr/bin/xcode-select"), &["-p"], &c.env.home, Duration::from_secs(5)).unwrap_or_default() } else { String::new() };
    let newest = xcodes.iter().map(|x| x.1.clone()).max().unwrap_or_default();
    let procs = c.procs();
    xcodes
        .into_iter()
        .filter(|(p, v)| *v < newest && !selected.trim().starts_with(&s(p)))
        .map(|(p, v)| {
            let size = du(&p);
            make("versions.xcode", Category::OldVersions, format!("Xcode {}", version_str(&v)), format!("app:{}", p.display()), vec![s(&p)], size, Safety::Review, false, format!("Xcode {} is installed too and this copy isn't the selected one.", version_str(&newest)))
                .busy(procs.app_running(&p).map(|x| format!("{} is running", who(x))))
        })
        .collect()
}

/// Simulators: devices whose runtime is gone, devices unused for 6 months, runtimes superseded
/// and unused.
pub fn simulators(c: &Clutter) -> Vec<Finding> {
    if !c.env.tools {
        return vec![];
    }
    let home = &c.env.home;
    let xcrun = Path::new("/usr/bin/xcrun");
    let devices_dir = home.join("Library/Developer/CoreSimulator/Devices");
    let mut out = vec![];
    let Some(json) = run(xcrun, &["simctl", "list", "devices", "--json"], home, Duration::from_secs(20)) else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else { return out };
    let mut dead = vec![];
    let mut stale = vec![];
    let mut used_runtimes: HashMap<String, i64> = HashMap::new();
    for (runtime, list) in v["devices"].as_object().into_iter().flatten() {
        for d in list.as_array().into_iter().flatten() {
            let Some(udid) = d["udid"].as_str() else { continue };
            let dir = devices_dir.join(udid);
            let name = d["name"].as_str().unwrap_or(udid).to_string();
            let booted_at = d["lastBootedAt"].as_str().and_then(parse_iso);
            if let Some(t) = booted_at {
                let e = used_runtimes.entry(runtime.clone()).or_insert(0);
                *e = (*e).max(t);
            }
            if d["state"].as_str() != Some("Shutdown") {
                used_runtimes.insert(runtime.clone(), now());
                continue;
            }
            if d["isAvailable"].as_bool() == Some(false) {
                dead.push((dir, name, udid.to_string()));
            } else {
                let created = crate::util::mtime(&dir.join("device.plist"));
                let last = booted_at.or(created);
                if last.is_some_and(|t| now() - t > 180 * DAY) {
                    stale.push((dir, name, udid.to_string(), last));
                }
            }
        }
    }
    if !dead.is_empty() {
        let paths: Vec<PathBuf> = dead.iter().map(|d| d.0.clone()).collect();
        let size = du_all(&paths);
        out.push(
            make("developer.simulators-unavailable", Category::Developer, "Broken simulators", "xcode", paths.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, "Simulators for iOS versions no longer installed; they can never boot again.")
                .keyed("simulators-unavailable")
                .detail(dead.iter().map(|d| d.1.clone()).collect::<Vec<_>>().join(", "))
                .command(&["/usr/bin/xcrun", "simctl", "delete", "unavailable"], "simctl delete unavailable"),
        );
    }
    if !stale.is_empty() {
        let paths: Vec<PathBuf> = stale.iter().map(|d| d.0.clone()).collect();
        let size = du_all(&paths);
        if size.bytes >= MIN {
            let mut argv = vec!["/usr/bin/xcrun".to_string(), "simctl".into(), "delete".into()];
            argv.extend(stale.iter().map(|d| d.2.clone()));
            let argv: Vec<&str> = argv.iter().map(|x| x.as_str()).collect();
            out.push(
                make("developer.simulators-unused", Category::Developer, "Unused simulators", "xcode", paths.iter().map(|p| s(p)).collect(), size, Safety::Review, false, "Simulators nobody booted in six months, with the test apps and data inside them. Xcode makes new ones on demand.")
                    .keyed("simulators-unused")
                    .detail(stale.iter().map(|d| d.1.clone()).collect::<Vec<_>>().join(", "))
                    .command(&argv, "simctl delete"),
            );
        }
    }
    // Runtimes: superseded by a newer one of the same platform and not used for 90 days.
    if let Some(rt) = run(xcrun, &["simctl", "runtime", "list", "-j"], home, Duration::from_secs(20)) {
        if let Ok(rt) = serde_json::from_str::<serde_json::Value>(&rt) {
            let list: Vec<&serde_json::Value> = rt.as_object().map(|o| o.values().collect()).unwrap_or_default();
            for r in &list {
                let platform = r["platformIdentifier"].as_str().unwrap_or("");
                let version = r["version"].as_str().and_then(parse_version).unwrap_or_default();
                let newer = list.iter().any(|o| o["platformIdentifier"].as_str() == Some(platform) && o["version"].as_str().and_then(parse_version).unwrap_or_default() > version);
                let rid = r["runtimeIdentifier"].as_str().unwrap_or("");
                let last = r["lastUsedAt"].as_str().and_then(parse_iso).max(used_runtimes.get(rid).copied());
                if !newer || r["deletable"].as_bool() != Some(true) || last.is_some_and(|t| now() - t < 90 * DAY) {
                    continue;
                }
                let Some(id) = r["identifier"].as_str() else { continue };
                let bytes = r["sizeBytes"].as_u64().unwrap_or(0);
                let path = r["path"].as_str().unwrap_or("").to_string();
                let name = format!("{} {} simulator", platform_name(platform), version_str(&version));
                out.push(
                    make("extras.simulator-runtime", Category::Extras, name, "xcode", vec![], Size { bytes, files: 0, newest: last }, Safety::Review, false, format!("A newer runtime for the same platform is installed and this one was last used {}. Xcode › Settings › Components can download it again.", ago(last)))
                        .keyed(&format!("runtime:{}", id))
                        .detail(path)
                        .command(&["/usr/bin/xcrun", "simctl", "runtime", "delete", id], "simctl runtime delete"),
                );
            }
        }
    }
    out
}

fn platform_name(id: &str) -> &'static str {
    if id.contains("iphone") {
        "iOS"
    } else if id.contains("watch") {
        "watchOS"
    } else if id.contains("appletv") {
        "tvOS"
    } else if id.contains("xr") {
        "visionOS"
    } else {
        "Simulator"
    }
}

/// "2026-11-06T18:58:47Z" → unix seconds.
pub fn parse_iso(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-11-06T18:58:47Z"), Some(1793991527));
    }

    #[test]
    fn support_versions() {
        assert_eq!(support_version("iPhone18,2 27.0 (24A435)"), ("iPhone18,2".into(), vec![27, 0]));
        assert_eq!(support_version("17.4 (21E219)"), ("".into(), vec![17, 4]));
        assert_eq!(derived_name("Disko-abcdefghijklmnopqrstuvwxyz"), "Disko");
        assert_eq!(derived_name("ModuleCache.noindex"), "ModuleCache.noindex");
    }
}
