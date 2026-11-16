//! Turning walk matches into findings a human can scan: one per project for build output, one per
//! app for caches, one per file for installers and forgotten files.

use crate::apps::is_bundle_id;
use crate::finding::{make, FindingExt};
use crate::procs::who;
use crate::rules::{DirRule, Grouping, PatternRule, FILE_RULES, FORGOTTEN_MIN_DAYS};
use crate::util::{ago, exists, human, name_of, now, s, Size, DAY, MB};
use crate::Clutter;
use disko_core::{paths::short, Category, Finding, Matched, Safety};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Running totals of one group (a project or an app).
#[derive(Debug, Default)]
pub(crate) struct Group {
    pub rule: String,
    pub root: PathBuf,
    /// path → (bytes, newest, rule index)
    pub members: BTreeMap<String, (u64, Option<i64>, usize)>,
}

const MANIFESTS: &[&str] = &[
    "package.json", "Cargo.toml", "Package.swift", "pyproject.toml", "setup.py", "requirements.txt", "Podfile", "build.gradle", "build.gradle.kts", "settings.gradle",
    "settings.gradle.kts", "pubspec.yaml", "go.mod", "mix.exs", "stack.yaml", "build.zig", "CMakeLists.txt", "elm.json", "project.yml",
];

/// Groups smaller than this aren't worth a line of their own.
const MIN_GROUP: u64 = MB;

const GENERIC: &[&str] = &[
    "src-tauri", "service", "services", "app", "apps", "web", "www", "frontend", "backend", "server", "client", "api", "packages", "macos", "ios", "android", "desktop",
    "mobile", "core", "src", "rust", "native", "platform", "site", "docs", "example", "examples", "demo", "crates", "lib", "libs", "tools", "scripts", "python", "node",
];

/// "my-app", "Disko › src-tauri", or "my-app · Codex worktree".
pub fn project_title(root: &Path) -> String {
    let mut name = name_of(root);
    if GENERIC.contains(&name.to_lowercase().as_str()) {
        if let Some(parent) = root.parent().map(name_of).filter(|p| !p.is_empty()) {
            name = format!("{} › {}", parent, name);
        }
    }
    let p = s(root);
    let origin = if p.contains("/.codex/worktrees/") {
        " · Codex worktree"
    } else if p.contains("/.claude/worktrees/") {
        " · Claude worktree"
    } else if p.contains("/.t3/worktrees/") {
        " · T3 worktree"
    } else if p.contains("/.cursor/worktrees/") {
        " · Cursor worktree"
    } else if p.contains("/.worktrees/") || p.contains("/conductor/") {
        " · worktree"
    } else {
        ""
    };
    format!("{}{}", name, origin)
}

impl Clutter {
    /// The project a build folder belongs to: the nearest git root (a worktree's `.git` file
    /// counts), else the folder's parent.
    pub(crate) fn project_root(&self, matched: &Path) -> PathBuf {
        let parent = matched.parent().unwrap_or(matched).to_path_buf();
        let home = &self.env.home;
        if let Some(r) = self.git_roots.lock().unwrap().get(&parent) {
            return r.clone();
        }
        // Nearest git root; else the nearest folder with a project manifest; else the parent.
        let mut git = None;
        let mut manifest = None;
        let mut cur = Some(parent.as_path());
        while let Some(d) = cur {
            if !d.starts_with(home) || d == home.as_path() || d.parent() == Some(home.as_path()) {
                break;
            }
            if exists(&d.join(".git")) {
                git = Some(d.to_path_buf());
                break;
            }
            if manifest.is_none() && MANIFESTS.iter().any(|m| exists(&d.join(m))) {
                manifest = Some(d.to_path_buf());
            }
            cur = d.parent();
        }
        let found = git.or(manifest).unwrap_or_else(|| parent.clone());
        self.git_roots.lock().unwrap().insert(parent, found.clone());
        found
    }

    pub(crate) fn dir_match(&self, rule_idx: usize, rule: &DirRule, m: &Matched) -> Option<Finding> {
        let path = PathBuf::from(&m.path);
        match rule.group {
            Grouping::Project => {
                let root = self.project_root(&path);
                // Environments need a decision; keep them apart from one-tap build output.
                let rule_id = if rule.safety == Safety::Review { "build.environments" } else { "build" };
                let key = format!("{}\u{0}{}", rule_id, s(&root));
                let mut groups = self.groups.lock().unwrap();
                let g = groups.entry(key).or_insert_with(|| Group { rule: rule_id.into(), root: root.clone(), ..Default::default() });
                g.members.insert(m.path.clone(), (m.bytes, m.newest, rule_idx));
                self.project_finding(g)
            }
            Grouping::App => {
                let (owner, owner_id) = self.app_owner(&path)?;
                // Data of an uninstalled app belongs to the leftovers probe, whole.
                if let Some(id) = &owner_id {
                    if self.apps().owner(id).is_none() && !id.starts_with("com.apple.") {
                        return None;
                    }
                }
                let rule_id = rule.id.to_string();
                let key = format!("{}\u{0}{}", rule_id, s(&owner));
                let mut groups = self.groups.lock().unwrap();
                let g = groups.entry(key).or_insert_with(|| Group { rule: rule_id.clone(), root: owner.clone(), ..Default::default() });
                g.members.insert(m.path.clone(), (m.bytes, m.newest, rule_idx));
                self.app_finding(g, rule, owner_id.as_deref())
            }
            Grouping::Single => {
                let f = make(rule.id, rule.category, name_of(&path), rule.icon, vec![m.path.clone()], Size { bytes: m.bytes, files: m.files, newest: m.newest }, rule.safety, rule.regrows, format!("{}; rebuilt by {}.", crate::util::cap(rule.kind), rule.rebuild));
                Some(f.detail(short(&m.path)))
            }
        }
    }

    fn project_finding(&self, g: &Group) -> Option<Finding> {
        let total: u64 = g.members.values().map(|v| v.0).sum();
        if total < MIN_GROUP {
            return None;
        }
        let newest = g.members.values().filter_map(|v| v.1).max();
        // Kinds, biggest first.
        let mut kinds: BTreeMap<&str, u64> = BTreeMap::new();
        let mut rebuild: BTreeMap<&str, u64> = BTreeMap::new();
        let mut icon_bytes: BTreeMap<&str, u64> = BTreeMap::new();
        let mut tools: Vec<&str> = vec![];
        for (b, _, i) in g.members.values() {
            let r = &crate::rules::DIR_RULES[*i];
            *kinds.entry(r.kind).or_default() += b;
            *rebuild.entry(r.rebuild).or_default() += b;
            *icon_bytes.entry(r.icon).or_default() += b;
            tools.extend(r.tools.iter());
        }
        let by_size = |m: BTreeMap<&str, u64>| {
            let mut v: Vec<(&str, u64)> = m.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v.into_iter().map(|x| x.0.to_string()).collect::<Vec<_>>()
        };
        let kinds = by_size(kinds);
        let rebuild = by_size(rebuild);
        let icon = by_size(icon_bytes).into_iter().next().unwrap_or_else(|| "box".into());
        let review = g.rule == "build.environments";
        let paths: Vec<String> = g.members.keys().cloned().collect();
        let n = paths.len();
        let why = if review {
            format!("{}; recreate with {}. Last changed {}.", if n == 1 { "A Python environment for this project" } else { "Environments for this project" }, rebuild.join(" / "), ago(newest))
        } else {
            format!("Generated by the build; {} brings it back. Last changed {}.", rebuild.into_iter().take(2).collect::<Vec<_>>().join(" / "), ago(newest))
        };
        let detail = if n > kinds.len() { format!("{} · {} folders", kinds.join(" · "), n) } else { kinds.join(" · ") };
        let safety = if review { Safety::Review } else { Safety::Safe };
        // Build output lives in the user's own folders: no admin check per emission.
        let mut f = make(&g.rule, Category::Build, crate::group::project_title(&g.root), icon, vec![], Size { bytes: total, files: 0, newest }, safety, true, why)
            .keyed(&s(&g.root))
            .detail(detail);
        f.paths = paths;
        let procs = self.procs();
        let busy = g
            .members
            .keys()
            .find_map(|p| procs.cwd_inside(Path::new(p)))
            .or_else(|| procs.tool_in(&g.root, &tools))
            .map(|p| format!("{} is running in this project", who(p)));
        Some(f.busy(busy))
    }

    /// Owner folder and its bundle id (if it is one) for an app-grouped match.
    fn app_owner(&self, path: &Path) -> Option<(PathBuf, Option<String>)> {
        let lib = self.env.home.join("Library");
        let rel = path.strip_prefix(&lib).ok()?;
        let c: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
        if c.len() < 2 {
            return None;
        }
        let (owner, id) = match c[0].as_str() {
            "Containers" | "Application Support" | "Caches" | "HTTPStorages" | "WebKit" => (lib.join(&c[0]).join(&c[1]), c[1].clone()),
            "Group Containers" => (lib.join(&c[0]).join(&c[1]), c[1].trim_start_matches("group.").to_string()),
            "Daemon Containers" if c.len() >= 2 => (lib.join(&c[0]).join(&c[1]), c[1].clone()),
            _ => (lib.join(&c[0]).join(&c[1]), c[1].clone()),
        };
        let id = if is_bundle_id(&id) { Some(id.to_ascii_lowercase()) } else { None };
        Some((owner, id))
    }

    fn app_finding(&self, g: &Group, rule: &DirRule, owner_id: Option<&str>) -> Option<Finding> {
        let total: u64 = g.members.values().map(|v| v.0).sum();
        if total < MIN_GROUP {
            return None;
        }
        let newest = g.members.values().filter_map(|v| v.1).max();
        let apps = self.apps();
        let folder = name_of(&g.root);
        let app = owner_id.and_then(|id| apps.owner(id)).or_else(|| apps.by_name(&folder));
        let title = app.map(|a| a.name.clone()).unwrap_or_else(|| {
            if owner_id.is_some() || folder.chars().any(|c| c.is_ascii_lowercase()) {
                crate::util::pretty_bundle(&folder)
            } else {
                // A UUID-named system container: say what it is instead.
                crate::util::cap(rule.kind)
            }
        });
        let icon = app.map(|a| format!("app:{}", a.path.display())).unwrap_or_else(|| rule.icon.to_string());
        let paths: Vec<String> = g.members.keys().cloned().collect();
        let names: Vec<String> = {
            let mut v: Vec<String> = paths.iter().map(|p| name_of(Path::new(p))).collect();
            v.sort();
            v.dedup();
            v
        };
        let why = match rule.id {
            "installers.sparkle" | "installers.shipit" => format!("Update files the app downloaded to install itself; {} fetches them again if needed.", rule.rebuild),
            "cache.coreml" => "Core ML models macOS compiled for this app; recompiled on next use, which makes that first run slower.".to_string(),
            _ => format!("Cache the app refills as you use it. Last written {}.", ago(newest)),
        };
        let mut f = make(&g.rule, rule.category, title, icon, vec![], Size { bytes: total, files: 0, newest }, rule.safety, rule.regrows, why)
            .keyed(&s(&g.root))
            .detail(format!("{} · {}", rule.kind, names.join(", ")));
        f.paths = paths;
        let procs = self.procs();
        let busy = app.and_then(|a| procs.app_running(&a.path)).map(|p| format!("{} is running", who(p)));
        Some(f.busy(busy))
    }

    pub(crate) fn file_match(&self, rule_idx: usize, m: &Matched) -> Option<Finding> {
        let rule = &FILE_RULES[rule_idx];
        let path = Path::new(&m.path);
        let name = name_of(path);
        let size = Size { bytes: m.bytes, files: m.files, newest: m.newest };
        let f = match rule.id {
            "installers.file" => {
                let stem = installer_stem(&name);
                let apps = self.apps();
                let app = apps.by_name(&stem);
                let why = match app {
                    Some(a) => format!("{} is installed{}; this installer already did its job.", a.name, a.version.as_ref().map(|v| format!(" ({})", v)).unwrap_or_default()),
                    None => format!("Installer or disk image from {}; download it again if you ever need it.", ago(m.newest)),
                };
                let icon = app.map(|a| format!("app:{}", a.path.display())).unwrap_or_else(|| rule.icon.into());
                make(rule.id, rule.category, if stem.is_empty() { name.clone() } else { stem }, icon, vec![m.path.clone()], size, rule.safety, false, why).detail(name)
            }
            "installers.partial" => make(rule.id, rule.category, name.clone(), rule.icon, vec![m.path.clone()], size, rule.safety, false, format!("A download that never finished, last touched {}.", ago(m.newest))),
            "logs.core-dump" => make(rule.id, rule.category, name.clone(), rule.icon, vec![m.path.clone()], size, rule.safety, false, "Memory dump of a crashed process; only useful to debug that crash.").detail(short(&m.path)),
            _ => {
                let days = m.newest.map(|t| (now() - t) / DAY).unwrap_or(0);
                if days < FORGOTTEN_MIN_DAYS {
                    return None;
                }
                make(rule.id, rule.category, name.clone(), rule.icon, vec![m.path.clone()], size, rule.safety, false, format!("{} that nobody changed {}.", human(m.bytes), ago(m.newest))).detail(short(&m.path))
            }
        };
        Some(f)
    }

    pub(crate) fn pattern_match(&self, rule: &PatternRule, path: &str, size: Size) -> Finding {
        let mut f = make(&rule.id, rule.category, rule.title.clone(), "box", vec![path.to_string()], size, rule.safety, rule.regrows, rule.why.clone()).detail(short(path));
        f.source = rule.source;
        f
    }
}

/// "Docker-4.32.0-arm64.dmg" → "Docker", "Install Xcode_16.xip" → "Xcode".
pub fn installer_stem(name: &str) -> String {
    let base = name.rsplit_once('.').map(|(b, _)| b).unwrap_or(name);
    let base = base.strip_prefix("Install ").unwrap_or(base);
    let mut words = vec![];
    for w in base.split([' ', '-', '_', '+']) {
        let lw = w.to_ascii_lowercase();
        if w.is_empty() || w.starts_with(|c: char| c.is_ascii_digit()) || w.starts_with(['v', 'V']) && w[1..].starts_with(|c: char| c.is_ascii_digit()) {
            break;
        }
        if ["arm64", "aarch64", "x64", "x86_64", "universal", "mac", "macos", "osx", "darwin", "setup", "installer", "apple", "silicon", "intel", "latest", "release"].contains(&lw.as_str()) {
            break;
        }
        words.push(w);
    }
    let s = words.join(" ");
    if s.chars().any(|c| c.is_uppercase()) || s.contains('.') {
        s
    } else {
        // "ableton live trial" → "Ableton Live Trial"
        s.split(' ').map(crate::util::cap).collect::<Vec<_>>().join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_names() {
        assert_eq!(installer_stem("Docker-4.32.0-arm64.dmg"), "Docker");
        assert_eq!(installer_stem("Install Xcode_16.xip"), "Xcode");
        assert_eq!(installer_stem("Visual Studio Code.dmg"), "Visual Studio Code");
        assert_eq!(installer_stem("OrbStack_v1.2.0_arm64.dmg"), "OrbStack");
        assert_eq!(installer_stem("googlechrome.dmg"), "Googlechrome");
        assert_eq!(installer_stem("ableton live trial.dmg"), "Ableton Live Trial");
    }

    #[test]
    fn titles() {
        assert_eq!(project_title(Path::new("/x/Disko/src-tauri")), "Disko › src-tauri");
        assert_eq!(project_title(Path::new("/Users/a/.codex/worktrees/ab12/proj")), "proj · Codex worktree");
    }
}
