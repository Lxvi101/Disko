//! Leftovers of apps that are no longer installed, and background items pointing at programs
//! that are gone.
//!
//! Ownership is an identity check, not a name guess: every bundle-id-named folder in the places
//! apps keep state is matched against the installed apps (exact id or dotted prefix either way),
//! running processes and package receipts. Apple's ids, password managers and security tools
//! are never proposed; shared vendor data and browser/chat profiles need stronger evidence.

use super::MIN;
use crate::apps::{is_bundle_id, vendor_key};
use crate::finding::{make, FindingExt};
use crate::util::{ago, children, du, exists, human, name_of, now, plist_dict, plist_str, pretty_bundle, run, s, Size, DAY};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Never proposed: Apple, credential stores, security products.
const VETO: &[&str] = &[
    "com.apple.", "com.1password", "com.agilebits", "com.bitwarden", "org.keepassx", "com.keepassxc", "com.lastpass", "com.dashlane", "in.sinew.enpass", "com.nordpass", "com.keepersecurity",
    "com.strongbox", "com.objective-see", "com.malwarebytes", "com.google.keystone", "com.microsoft.autoupdate", "org.sparkle-project",
];

/// Profiles, mail and chat persist past uninstall on purpose.
const SENSITIVE: &[&str] = &[
    "org.mozilla", "com.google.chrome", "com.microsoft.edgemac", "com.brave.browser", "company.thebrowser", "com.vivaldi", "com.operasoftware", "com.tinyspeck", "com.hnc.discord",
    "ru.keepcoder.telegram", "net.whatsapp", "com.microsoft.teams", "org.whispersystems.signal", "com.readdle.smartemail", "com.microsoft.outlook", "com.freron.mailmate",
];

struct Root {
    dir: &'static str,
    kind: &'static str,
    suffix: &'static str,
    persistent: bool,
}

const ROOTS: &[Root] = &[
    Root { dir: "Library/Caches", kind: "cache", suffix: "", persistent: false },
    Root { dir: "Library/HTTPStorages", kind: "http", suffix: "", persistent: false },
    Root { dir: "Library/HTTPStorages", kind: "http", suffix: ".binarycookies", persistent: false },
    Root { dir: "Library/Saved Application State", kind: "saved state", suffix: ".savedState", persistent: false },
    Root { dir: "Library/Logs", kind: "logs", suffix: "", persistent: false },
    Root { dir: "Library/WebKit", kind: "web data", suffix: "", persistent: true },
    Root { dir: "Library/Application Support", kind: "data", suffix: "", persistent: true },
    Root { dir: "Library/Containers", kind: "container", suffix: "", persistent: true },
    Root { dir: "Library/Group Containers", kind: "group container", suffix: "", persistent: true },
    Root { dir: "Library/Preferences", kind: "settings", suffix: ".plist", persistent: true },
    Root { dir: "Library/Application Scripts", kind: "scripts", suffix: "", persistent: true },
    Root { dir: "Library/Cookies", kind: "cookies", suffix: ".binarycookies", persistent: true },
    Root { dir: "Library/Preferences/ByHost", kind: "settings", suffix: "", persistent: true },
];

/// "group.com.foo.bar" / "ABCDE12345.com.foo.bar" → "com.foo.bar".
pub fn group_identity(name: &str) -> Option<String> {
    let t = name.strip_prefix("group.").unwrap_or(name);
    let t = match t.split_once('.') {
        Some((team, rest)) if team.len() == 10 && team.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) => rest,
        _ => t,
    };
    let t = t.strip_prefix("group.").unwrap_or(t);
    is_bundle_id(t).then(|| t.to_string())
}

/// "com.foo.bar.ABCDEF12-…-.plist" in ByHost → "com.foo.bar".
fn byhost_identity(name: &str) -> Option<String> {
    let base = name.strip_suffix(".plist")?;
    let (id, _uuid) = base.rsplit_once('.')?;
    is_bundle_id(id).then(|| id.to_string())
}

fn receipts(c: &Clutter) -> HashSet<String> {
    if !c.env.tools {
        return HashSet::new();
    }
    run(Path::new("/usr/sbin/pkgutil"), &["--pkgs"], &c.env.home, Duration::from_secs(10)).map(|o| o.lines().map(|l| l.trim().to_ascii_lowercase()).collect()).unwrap_or_default()
}

pub fn probe(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let apps = c.apps();
    let procs = c.procs();
    let receipts = receipts(c);
    // Bundle ids of running apps (their bundle may live somewhere we didn't look).
    let running: HashSet<String> = procs
        .list
        .iter()
        .filter_map(|p| {
            let i = p.exe.find(".app/Contents/")?;
            crate::apps::read_app(Path::new(&p.exe[..i + 4])).map(|a| a.id)
        })
        .collect();
    let live = |id: &str| running.iter().any(|r| r == id || id.starts_with(&format!("{}.", r)) || r.starts_with(&format!("{}.", id)));

    let mut by_id: BTreeMap<String, Vec<(PathBuf, &'static str, bool)>> = BTreeMap::new();
    for root in ROOTS {
        if !c.env.readable(&home.join(root.dir)) {
            continue;
        }
        for p in children(&home.join(root.dir)) {
            let name = name_of(&p);
            if !root.suffix.is_empty() && !name.ends_with(root.suffix) {
                continue;
            }
            let base = name.strip_suffix(root.suffix).unwrap_or(&name);
            let id = match root.kind {
                "group container" => group_identity(base),
                _ if root.dir.ends_with("ByHost") => byhost_identity(&name),
                _ => is_bundle_id(base).then(|| base.to_string()),
            };
            let Some(id) = id else { continue };
            let key = id.to_ascii_lowercase();
            if VETO.iter().any(|v| key.starts_with(v)) {
                continue;
            }
            by_id.entry(key).or_default().push((p, root.kind, root.persistent));
        }
    }

    // An app's extensions and helpers ("com.foo.app.quicklook") go with the app.
    let ids: Vec<String> = by_id.keys().cloned().collect();
    for id in &ids {
        if let Some(parent) = ids.iter().filter(|p| id.starts_with(&format!("{}.", p))).min_by_key(|p| p.len()) {
            if let Some(m) = by_id.remove(id) {
                by_id.entry(parent.clone()).or_default().extend(m);
            }
        }
    }
    by_id
        .into_par_iter()
        .filter_map(|(id, members)| {
            if apps.owner(&id).is_some() || live(&id) {
                return None;
            }
            let mut evidence = vec![format!("no installed app has the id {}", id)];
            let mut score = 7;
            let mut sizes = vec![];
            let mut total = Size::default();
            for (p, kind, persistent) in &members {
                let sz = du(p);
                total.add(sz);
                sizes.push((p.clone(), *kind, *persistent, sz));
            }
            let stale = total.newest.is_some_and(|n| now() - n >= 90 * DAY);
            if stale {
                score += 1;
            } else {
                score -= 2;
                evidence.push(format!("something wrote to it {}", ago(total.newest)));
            }
            let app_evidence = sizes.iter().any(|m| matches!(m.1, "settings" | "saved state" | "container" | "data" | "web data" | "group container" | "scripts"));
            let had_receipt = receipts.iter().any(|r| r == &id || r.starts_with(&format!("{}.", id)));
            if !app_evidence && !had_receipt {
                // A bare cache folder may belong to a command-line tool or library.
                score -= 3;
                if total.bytes < 50 * MIN {
                    return None;
                }
            }
            if apps.vendor_installed(&id) {
                score -= 3;
                evidence.push(format!("other {} apps are installed and may share it", vendor_key(&id)));
            }
            if sizes.iter().any(|m| m.1 == "group container") {
                score -= 2;
            }
            if SENSITIVE.iter().any(|x| id.starts_with(x)) {
                score -= 4;
                evidence.push("browser, mail and chat data is kept on purpose".into());
            }
            if score < 4 {
                return None;
            }
            // Small files matter (the principle), but a lone empty prefs file isn't worth a line.
            if total.bytes < 64 * 1024 && sizes.len() < 2 {
                return None;
            }
            let kinds: Vec<&str> = {
                let mut k: Vec<&str> = sizes.iter().map(|m| m.1).collect();
                k.sort();
                k.dedup();
                k
            };
            let name = pretty_bundle(&id);
            let why = format!(
                "{} left behind: {}{}.",
                kinds.join(", "),
                evidence.join("; "),
                if had_receipt { "; an installer receipt shows it was installed as a package" } else { "" }
            );
            let paths: Vec<String> = sizes.iter().map(|m| s(&m.0)).collect();
            Some(
                make("leftovers.app", Category::Leftovers, crate::util::cap(&name), "app", paths, total, Safety::Review, false, crate::util::cap(&why))
                    .keyed(&format!("app:{}", id))
                    .detail(format!("{} · {} items", id, sizes.len())),
            )
        })
        .collect()
}

fn program_of(d: &plist::Dictionary) -> Option<String> {
    plist_str(d, "Program").or_else(|| d.get("ProgramArguments")?.as_array()?.first()?.as_string().map(String::from))
}

/// Launch agents/daemons whose program no longer exists, empty Keystone plists, and broken JDK
/// registrations.
pub fn launch_items(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let mut user_dead = vec![];
    let mut sys_dead = vec![];
    let mut labels = vec![];
    for (dir, system) in [(home.join("Library/LaunchAgents"), false), (c.env.sys("/Library/LaunchAgents"), true), (c.env.sys("/Library/LaunchDaemons"), true)] {
        for p in children(&dir) {
            let n = name_of(&p);
            if !n.ends_with(".plist") || n.starts_with("com.apple.") {
                continue;
            }
            let meta = fs::symlink_metadata(&p);
            // Google's Keystone leaves zero-byte agents behind after uninstalling Chrome.
            let empty = meta.as_ref().map(|m| m.len() == 0).unwrap_or(false);
            let dead = if empty {
                true
            } else {
                match plist_dict(&p).as_ref().and_then(program_of) {
                    Some(prog) if prog.starts_with('/') => !exists(Path::new(&prog)) && !exists(&c.env.sys(&prog)),
                    _ => false,
                }
            };
            if dead {
                labels.push(n.trim_end_matches(".plist").to_string());
                if system {
                    sys_dead.push(p)
                } else {
                    user_dead.push(p)
                }
            }
        }
    }
    let mut out = vec![];
    for (list, rule, admin_note) in [(user_dead, "leftovers.launch-agents", ""), (sys_dead, "leftovers.launch-daemons", " System-wide, so it needs your password.")] {
        if list.is_empty() {
            continue;
        }
        let size = crate::util::du_all(&list);
        let names: Vec<String> = list.iter().map(|p| name_of(p).trim_end_matches(".plist").to_string()).collect();
        out.push(
            make(rule, Category::Leftovers, if rule.ends_with("agents") { "Dead launch agents" } else { "Dead system launch items" }, "app", list.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("{} background item(s) start a program that no longer exists (or are empty files), so launchd fails on every login.{}", list.len(), admin_note))
                .keyed(rule)
                .detail(names.join(", ")),
        );
    }
    // JDKs registered with macOS whose install is gone.
    let jvm = c.env.sys("/Library/Java/JavaVirtualMachines");
    let broken: Vec<PathBuf> = children(&jvm).into_iter().filter(|p| fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false) && fs::metadata(p).is_err()).collect();
    if !broken.is_empty() {
        out.push(
            make("leftovers.jdk-links", Category::Leftovers, "Broken Java registrations", "java", broken.iter().map(|p| s(p)).collect(), Size { bytes: 0, files: broken.len() as u64, newest: None }, Safety::Review, false, "Links in /Library/Java/JavaVirtualMachines to JDKs that were removed; `java_home` still lists them and tools pick a JDK that isn't there.")
                .keyed("jdk-links")
                .detail(broken.iter().map(|p| name_of(p)).collect::<Vec<_>>().join(", ")),
        );
    }
    let _ = human;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_ids() {
        assert_eq!(group_identity("group.com.foo.bar").as_deref(), Some("com.foo.bar"));
        assert_eq!(group_identity("ABCDE12345.com.foo.bar").as_deref(), Some("com.foo.bar"));
        assert_eq!(group_identity("ABCDE12345.group.com.foo.bar").as_deref(), Some("com.foo.bar"));
        assert_eq!(group_identity("Foo"), None);
        assert_eq!(byhost_identity("com.foo.bar.12345678-AAAA-BBBB-CCCC-1234567890AB.plist").as_deref(), Some("com.foo.bar"));
    }
}
