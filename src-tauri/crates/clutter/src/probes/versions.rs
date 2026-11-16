//! Older versions superseded by the one in use: language runtimes, toolchains, IDE versions, test
//! browsers, agent CLI releases and editor extensions.

use super::MIN;
use crate::finding::{make, FindingExt};
use crate::procs::who;
use crate::util::{ago, child_dirs, children, du, du_all, exists, is_dir, name_of, parse_version, s, tool, trailing_version, version_str};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Version pins found in projects (.nvmrc, rust-toolchain …), from a bounded walk of the usual
/// project folders.
#[derive(Debug, Default)]
pub struct Pins {
    pub node: Vec<String>,
    pub rust: Vec<String>,
    pub python: Vec<String>,
}

const DEV_ROOTS: &[&str] = &[
    "Documents/Development", "Development", "Developer", "Projects", "projects", "Code", "code", "src", "dev", "Dev", "repos", "Repos", "GitHub", "git", "workspace", "Workspace", "Sites",
    "Documents/Codex", "Documents/GitHub", "Documents/Projects", "Documents/Code", "Desktop", ".codex/worktrees", ".t3/worktrees", ".claude/worktrees", "conductor", ".cursor/worktrees",
];
const SKIP: &[&str] = &["node_modules", "target", ".git", ".build", "DerivedData", "Pods", ".venv", "venv", "__pycache__", "dist", "build", ".next", "vendor", ".gradle", "Library"];

pub fn pins(c: &Clutter) -> Pins {
    let home = &c.env.home;
    fn walk(dir: &Path, depth: u8, out: &mut Pins) {
        for p in children(dir) {
            let n = name_of(&p);
            if is_dir(&p) {
                if depth > 0 && !SKIP.contains(&n.as_str()) && !n.ends_with(".app") {
                    walk(&p, depth - 1, out);
                }
                continue;
            }
            let read = || fs::read_to_string(&p).unwrap_or_default();
            match n.as_str() {
                ".nvmrc" | ".node-version" => out.node.push(read().trim().trim_start_matches('v').to_string()),
                ".python-version" => out.python.extend(read().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())),
                "rust-toolchain" => out.rust.push(read().trim().to_string()),
                "rust-toolchain.toml" => {
                    for line in read().lines() {
                        if let Some(v) = line.trim().strip_prefix("channel") {
                            out.rust.push(v.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string());
                        }
                    }
                }
                ".tool-versions" => {
                    for line in read().lines() {
                        let mut it = line.split_whitespace();
                        match (it.next(), it.next()) {
                            (Some("nodejs"), Some(v)) => out.node.push(v.into()),
                            (Some("python"), Some(v)) => out.python.push(v.into()),
                            (Some("rust"), Some(v)) => out.rust.push(v.into()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Pins::default();
    let mut seen = HashSet::new();
    for r in DEV_ROOTS {
        let p = home.join(r);
        if c.env.readable(&p) && is_dir(&p) && seen.insert(fs::canonicalize(&p).unwrap_or(p.clone())) {
            walk(&p, if r.starts_with('.') || *r == "Desktop" { 4 } else { 6 }, &mut out);
        }
    }
    out
}

/// A pin like "20" or "20.11" or "lts/iron" matches version [20, 11, 1] by prefix.
fn pin_matches(pin: &str, v: &[u64]) -> bool {
    parse_version(pin).is_some_and(|p| !p.is_empty() && v.starts_with(&p))
}

pub fn probe(c: &Clutter) -> Vec<Finding> {
    let pins = pins(c);
    let jobs: Vec<Box<dyn Fn() -> Vec<Finding> + Sync>> = vec![
        Box::new(|| node(c, &pins)),
        Box::new(|| python(c, &pins)),
        Box::new(|| rust(c, &pins)),
        Box::new(|| jetbrains(c)),
        Box::new(|| browsers(c)),
    ];
    jobs.par_iter().flat_map(|j| j()).collect()
}

fn running_from(c: &Clutter, dirs: &[PathBuf]) -> Option<String> {
    let procs = c.procs();
    dirs.iter().find_map(|d| procs.exe_inside(d)).map(|p| format!("{} is running from it", who(p)))
}

/// Node versions from nvm, fnm and Volta beside the default.
fn node(c: &Clutter, pins: &Pins) -> Vec<Finding> {
    let home = &c.env.home;
    let mut out = vec![];
    let managers: [(&str, PathBuf, Option<String>); 3] = [
        ("nvm", home.join(".nvm/versions/node"), fs::read_to_string(home.join(".nvm/alias/default")).ok()),
        ("fnm", if is_dir(&home.join(".local/share/fnm/node-versions")) { home.join(".local/share/fnm/node-versions") } else { home.join("Library/Application Support/fnm/node-versions") }, fs::read_link(home.join(".local/share/fnm/aliases/default")).ok().map(|p| name_of(&p))),
        ("Volta", home.join(".volta/tools/image/node"), fs::read_to_string(home.join(".volta/tools/user/platform.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v["node"]["runtime"].as_str().map(String::from))),
    ];
    for (mgr, root, default) in managers {
        let mut versions: Vec<(Vec<u64>, PathBuf)> = child_dirs(&root).into_iter().filter_map(|p| Some((parse_version(&name_of(&p))?, p))).collect();
        if versions.len() < 2 {
            continue;
        }
        versions.sort();
        let default = default.unwrap_or_default();
        let default = default.trim().trim_start_matches('v');
        // "node"/"stable"/"lts/*"/"" mean the newest; "20" means the newest 20.x.
        let keep_default = versions.iter().rev().find(|(v, _)| pin_matches(default, v)).or(versions.last()).map(|x| x.0.clone()).unwrap_or_default();
        let old: Vec<&(Vec<u64>, PathBuf)> = versions.iter().filter(|(v, _)| *v != keep_default && !pins.node.iter().any(|p| pin_matches(p, v))).collect();
        let procs = c.procs();
        let old: Vec<&(Vec<u64>, PathBuf)> = old.into_iter().filter(|(_, p)| procs.exe_inside(p).is_none()).collect();
        if old.is_empty() {
            continue;
        }
        let paths: Vec<PathBuf> = old.iter().map(|x| x.1.clone()).collect();
        let size = du_all(&paths);
        if size.bytes < MIN {
            continue;
        }
        let names: Vec<String> = old.iter().map(|x| format!("v{}", version_str(&x.0))).collect();
        out.push(
            make("versions.node", Category::OldVersions, format!("Node.js ({})", mgr), "node", paths.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("v{} is your default; no project pins {} with .nvmrc or .node-version. Global packages installed in them go too.", version_str(&keep_default), names.join(", ")))
                .keyed(&s(&root))
                .detail(names.join(", ")),
        );
    }
    out
}

/// pyenv versions beside the global one; uv-managed Pythons beside a newer patch.
fn python(c: &Clutter, pins: &Pins) -> Vec<Finding> {
    let home = &c.env.home;
    let mut out = vec![];
    let root = home.join(".pyenv/versions");
    let global: Vec<String> = fs::read_to_string(home.join(".pyenv/version")).unwrap_or_default().lines().map(|l| l.trim().to_string()).collect();
    let mut old = vec![];
    for p in child_dirs(&root) {
        let n = name_of(&p);
        if global.contains(&n) || pins.python.iter().any(|x| x == &n || n.starts_with(&format!("{}.", x))) {
            continue;
        }
        old.push(p);
    }
    if !old.is_empty() && child_dirs(&root).len() > old.len() {
        let size = du_all(&old);
        if size.bytes >= MIN {
            let names: Vec<String> = old.iter().map(|p| name_of(p)).collect();
            out.push(
                make("versions.pyenv", Category::OldVersions, "Python (pyenv)", "python", old.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("{} is your global Python; nothing pins {} with .python-version.", global.join(", "), names.join(", ")))
                    .keyed(&s(&root))
                    .detail(names.join(", "))
                    .busy(running_from(c, &old)),
            );
        }
    }
    // uv: cpython-3.12.4-macos-aarch64-none beside cpython-3.12.8-…
    let uvroot = home.join(".local/share/uv/python");
    let mut by_minor: BTreeMap<(String, u64, u64), Vec<(Vec<u64>, PathBuf)>> = BTreeMap::new();
    for p in child_dirs(&uvroot) {
        let n = name_of(&p);
        let Some((imp, rest)) = n.split_once('-') else { continue };
        let Some(v) = parse_version(rest) else { continue };
        if v.len() >= 3 {
            by_minor.entry((imp.to_string(), v[0], v[1])).or_default().push((v, p));
        }
    }
    let uv = tool(home, "uv");
    for ((imp, a, b), mut list) in by_minor {
        if list.len() < 2 {
            continue;
        }
        list.sort();
        let newest = list.pop().unwrap();
        for (v, p) in list {
            let size = du(&p);
            if size.bytes < MIN {
                continue;
            }
            let ver = version_str(&v);
            let mut f = make("versions.uv-python", Category::OldVersions, format!("Python {}", ver), "python", vec![s(&p)], size, Safety::Review, false, format!("uv also has {} {}.{} patch {}; virtual environments made with {} stop working until recreated.", imp, a, b, version_str(&newest.0), ver)).detail("uv-managed Python");
            if let Some(uv) = &uv {
                f = f.command(&[&s(uv), "python", "uninstall", &ver], &format!("uv python uninstall {}", ver));
            }
            out.push(f.busy(running_from(c, &[p])));
        }
    }
    out
}

/// "1.93-aarch64-apple-darwin" is pinned by "1.93"; "stable-…" by "stable".
fn toolchain_pinned(name: &str, pins: &[String]) -> bool {
    pins.iter().any(|p| !p.is_empty() && (name == p || name.starts_with(&format!("{}-", p))))
}

fn valid_toolchain(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
}

/// Rust toolchains that aren't the default or pinned by a project, and offline docs.
fn rust(c: &Clutter, pins: &Pins) -> Vec<Finding> {
    let home = &c.env.home;
    let root = home.join(".rustup/toolchains");
    let toolchains = child_dirs(&root);
    if toolchains.is_empty() {
        return vec![];
    }
    let settings = fs::read_to_string(home.join(".rustup/settings.toml")).unwrap_or_default();
    let mut keep: Vec<String> = vec![];
    let mut in_overrides = false;
    for line in settings.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_overrides = l == "[overrides]";
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            if in_overrides || k.trim() == "default_toolchain" {
                keep.push(v);
            }
        }
    }
    keep.extend(pins.rust.iter().cloned());
    let rustup = tool(home, "rustup");
    let procs = c.procs();
    let mut out = vec![];
    let mut removing = HashSet::new();
    for t in &toolchains {
        let n = name_of(t);
        if toolchain_pinned(&n, &keep) || !valid_toolchain(&n) || n.starts_with("stable") || n.starts_with("nightly") || n.starts_with("beta") {
            continue;
        }
        let size = du(t);
        if size.bytes < MIN {
            continue;
        }
        removing.insert(n.clone());
        let busy = procs.exe_inside(t).map(|p| format!("{} is running from it", who(p)));
        let mut f = make("versions.rust-toolchain", Category::OldVersions, format!("Rust {}", n.split('-').next().unwrap_or(&n)), "rust", vec![s(t)], size, Safety::Review, false, format!("Not your default toolchain and no rust-toolchain file in your projects pins it; last used {}.", ago(size.newest)))
            .detail(n.clone())
            .busy(busy);
        if let Some(r) = &rustup {
            f = f.command(&[&s(r), "toolchain", "uninstall", &n], &format!("rustup toolchain uninstall {}", n));
        }
        out.push(f);
    }
    // Offline HTML docs, a separate component, in toolchains that stay.
    let mut docs = vec![];
    let mut names = vec![];
    for t in &toolchains {
        let n = name_of(t);
        if removing.contains(&n) || !valid_toolchain(&n) {
            continue;
        }
        let p = t.join("share/doc/rust");
        let listed = fs::read_to_string(t.join("lib/rustlib/components")).map(|s| s.lines().any(|l| l == "rust-docs" || l.starts_with("rust-docs-"))).unwrap_or(false);
        if listed && is_dir(&p) {
            docs.push(p);
            names.push(n);
        }
    }
    if let (false, Some(r)) = (docs.is_empty(), &rustup) {
        let size = du_all(&docs);
        if size.bytes >= MIN {
            let script = names.iter().map(|n| format!("'{}' component remove rust-docs --toolchain '{}'", s(r), n)).collect::<Vec<_>>().join(" && ");
            out.push(
                make("extras.rust-docs", Category::Extras, "Rust offline docs", "rust", docs.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, format!("The optional rust-docs component in {} toolchain(s); compilers and targets stay. `rustup component add rust-docs` brings it back.", names.len()))
                    .keyed("rust-docs")
                    .detail(names.join(", "))
                    .command(&["/bin/sh", "-c", &script], "rustup component remove rust-docs"),
            );
        }
    }
    out
}

/// JetBrains keeps settings, caches and logs per IDE version; versions older than the newest
/// of the same product are left behind after upgrades.
fn jetbrains(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let roots = [home.join("Library/Application Support/JetBrains"), home.join("Library/Caches/JetBrains"), home.join("Library/Logs/JetBrains")];
    let mut by_product: HashMap<String, Vec<(Vec<u64>, PathBuf)>> = HashMap::new();
    for r in &roots {
        for p in child_dirs(r) {
            if let Some((prod, v)) = trailing_version(&name_of(&p)) {
                by_product.entry(prod).or_default().push((v, p));
            }
        }
    }
    let mut out = vec![];
    for (prod, list) in by_product {
        let newest = list.iter().map(|x| x.0.clone()).max().unwrap_or_default();
        let old: Vec<PathBuf> = list.iter().filter(|x| x.0 < newest).map(|x| x.1.clone()).collect();
        if old.is_empty() {
            continue;
        }
        let size = du_all(&old);
        if size.bytes < MIN {
            continue;
        }
        let mut vs: Vec<String> = list.iter().filter(|x| x.0 < newest).map(|x| version_str(&x.0)).collect();
        vs.sort();
        vs.dedup();
        out.push(
            make("versions.jetbrains", Category::OldVersions, prod.clone(), "app", old.iter().map(|p| s(p)).collect(), size, Safety::Review, false, format!("Settings, caches and logs of {} {}; {} is the newest and already imported its settings.", prod, vs.join(", "), version_str(&newest)))
                .keyed(&format!("jetbrains:{}", prod))
                .detail(format!("{} old versions", vs.len())),
        );
    }
    out
}

/// Test browsers: Playwright, puppeteer and Cypress keep every revision they downloaded.
fn browsers(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let mut out = vec![];
    for root in [home.join("Library/Caches/ms-playwright"), home.join(".cache/ms-playwright")] {
        // "chromium-1254", "chromium_headless_shell-1254", "webkit-2388"
        let mut by: HashMap<String, Vec<(u64, PathBuf)>> = HashMap::new();
        for p in child_dirs(&root) {
            let n = name_of(&p);
            if let Some((b, rev)) = n.rsplit_once('-') {
                if let Ok(r) = rev.parse::<u64>() {
                    by.entry(b.to_string()).or_default().push((r, p));
                }
            }
        }
        let old: Vec<PathBuf> = by.values().flat_map(|l| {
            let max = l.iter().map(|x| x.0).max().unwrap_or(0);
            l.iter().filter(move |x| x.0 < max).map(|x| x.1.clone())
        }).collect();
        if let Some(f) = old_set(c, "versions.playwright", "Playwright browsers", &root, old, "Browser builds an older Playwright downloaded; the newest revision of each stays.") {
            out.push(f);
        }
    }
    // puppeteer: ~/.cache/puppeteer/<browser>/<platform>-<build>
    let pr = home.join(".cache/puppeteer");
    let mut old = vec![];
    for b in child_dirs(&pr) {
        let mut l: Vec<(Vec<u64>, PathBuf)> = child_dirs(&b).into_iter().filter_map(|p| Some((parse_version(name_of(&p).rsplit('-').next()?)?, p))).collect();
        l.sort();
        l.pop();
        old.extend(l.into_iter().map(|x| x.1));
    }
    if let Some(f) = old_set(c, "versions.puppeteer", "Puppeteer browsers", &pr, old, "Browser builds an older puppeteer downloaded; the newest of each stays.") {
        out.push(f);
    }
    let cy = home.join("Library/Caches/Cypress");
    let mut l: Vec<(Vec<u64>, PathBuf)> = child_dirs(&cy).into_iter().filter_map(|p| Some((parse_version(&name_of(&p))?, p))).collect();
    l.sort();
    l.pop();
    if let Some(f) = old_set(c, "versions.cypress", "Cypress", &cy, l.into_iter().map(|x| x.1).collect(), "Cypress app versions older projects installed; the newest stays.") {
        out.push(f);
    }
    out
}

fn old_set(c: &Clutter, rule: &str, title: &str, root: &Path, old: Vec<PathBuf>, why: &str) -> Option<Finding> {
    if old.is_empty() {
        return None;
    }
    let size = du_all(&old);
    if size.bytes < MIN {
        return None;
    }
    let names: Vec<String> = old.iter().map(|p| name_of(p)).collect();
    Some(make(rule, Category::OldVersions, title, "box", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, why).keyed(&s(root)).detail(names.join(", ")).busy(running_from(c, &old)))
}

/// Folders that hold launchers on PATH.
fn bin_dirs(c: &Clutter) -> Vec<PathBuf> {
    let h = &c.env.home;
    vec![h.join(".local/bin"), h.join("bin"), c.env.sys("/usr/local/bin"), h.join(".bun/bin"), h.join(".cargo/bin")]
}

/// Directories that are already handled elsewhere or are app internals.
fn skip_root(c: &Clutter, root: &Path) -> bool {
    let h = &c.env.home;
    let r = s(root);
    r.contains("/Cellar/") || r.contains("/Caskroom/") || r.contains(".app/") || root.starts_with(h.join(".nvm")) || root.starts_with(h.join(".rustup")) || root.starts_with(h.join(".pyenv")) || root.starts_with(h.join(".volta"))
}

/// The versions folder a resolved launcher target lives in: the nearest ancestor named like a
/// version whose parent holds at least one other version.
fn versions_root(target: &Path) -> Option<(PathBuf, PathBuf, Vec<u64>)> {
    for a in target.ancestors().take(6) {
        let Some(v) = parse_version(&name_of(a)) else { continue };
        let parent = a.parent()?;
        let siblings = child_dirs(parent).into_iter().chain(children(parent).into_iter().filter(|p| crate::util::is_file(p))).filter(|p| p != a && parse_version(&name_of(p)).is_some()).count();
        if siblings >= 1 {
            return Some((parent.to_path_buf(), a.to_path_buf(), v));
        }
    }
    None
}

/// Superseded releases: anything installed in a versions folder that a launcher (a symlink on
/// PATH, or a `current` pointer) no longer points into, and that is older than what it points to.
/// Covers Claude Code, Codex, cursor-agent, GitKraken CLI, Ardia and any tool installed the same way.
pub fn releases(c: &Clutter) -> Vec<Finding> {
    let h = &c.env.home;
    let mut pointers: Vec<(String, PathBuf)> = vec![];
    for d in bin_dirs(c) {
        for p in children(&d) {
            if fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                pointers.push((name_of(&p), p));
            }
        }
    }
    for (label, p) in [("codex", ".codex/packages/standalone/current"), ("codex daemon", ".codex/packages/app-server-daemon/current"), ("gk", ".gitkraken/cli/current")] {
        pointers.push((label.into(), h.join(p)));
    }
    // root → (label, current versions in use)
    let mut roots: BTreeMap<PathBuf, (String, Vec<(PathBuf, Vec<u64>)>)> = BTreeMap::new();
    for (label, p) in &pointers {
        let Ok(target) = fs::canonicalize(p) else { continue };
        let Some((root, current, v)) = versions_root(&target) else { continue };
        if skip_root(c, &root) {
            continue;
        }
        let e = roots.entry(root).or_insert_with(|| (label.clone(), vec![]));
        e.1.push((current, v));
    }
    let procs = c.procs();
    let mut out = vec![];
    for (root, (label, current)) in roots {
        let newest_current = current.iter().map(|x| x.1.clone()).max().unwrap_or_default();
        let old: Vec<PathBuf> = children(&root)
            .into_iter()
            .filter(|p| !fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(true))
            .filter(|p| parse_version(&name_of(p)).is_some_and(|v| v < newest_current))
            .filter(|p| !current.iter().any(|(cur, _)| cur == p))
            .filter(|p| procs.exe_inside(p).is_none() && procs.app_running(p).is_none())
            .collect();
        if old.is_empty() {
            continue;
        }
        let size = du_all(&old);
        if size.bytes < MIN {
            continue;
        }
        let names: Vec<String> = old.iter().map(|p| name_of(p)).collect();
        let title = match label.as_str() {
            "claude" => "Claude Code".to_string(),
            "codex" | "codex daemon" | "codex-code-mode-host" => "Codex CLI".to_string(),
            "agent" | "cursor-agent" => "Cursor Agent".to_string(),
            "gk" => "GitKraken CLI".to_string(),
            l => crate::util::cap(l),
        };
        out.push(
            make("versions.releases", Category::OldVersions, title, "box", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, format!("`{}` runs version {}; these older releases beside it aren't used by anything running.", label, version_str(&newest_current)))
                .keyed(&s(&root))
                .detail(names.join(", ")),
        );
    }
    out
}

/// Editor extension folders VS Code-style editors no longer load: listed in `.obsolete`, or
/// absent from `extensions.json` while a newer copy of the same extension is.
pub fn extensions(c: &Clutter) -> Vec<Finding> {
    let h = &c.env.home;
    let mut out = vec![];
    for (rel, editor, app) in [
        (".vscode/extensions", "Visual Studio Code", "Visual Studio Code"),
        (".vscode-insiders/extensions", "VS Code Insiders", "Visual Studio Code - Insiders"),
        (".cursor/extensions", "Cursor", "Cursor"),
        (".windsurf/extensions", "Windsurf", "Windsurf"),
        (".vscode-oss/extensions", "VSCodium", "VSCodium"),
        (".antigravity/extensions", "Antigravity", "Antigravity"),
        (".kiro/extensions", "Kiro", "Kiro"),
    ] {
        let root = h.join(rel);
        if !is_dir(&root) {
            continue;
        }
        let obsolete: HashSet<String> = fs::read_to_string(root.join(".obsolete")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.as_object().map(|o| o.keys().cloned().collect())).unwrap_or_default();
        let listed: Option<HashSet<String>> = fs::read_to_string(root.join("extensions.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| {
            Some(v.as_array()?.iter().filter_map(|e| e["relativeLocation"].as_str().map(String::from).or_else(|| e["location"]["path"].as_str().map(|p| name_of(Path::new(p))))).collect())
        });
        let dirs = child_dirs(&root);
        let mut old = vec![];
        for d in &dirs {
            let n = name_of(d);
            if n.starts_with('.') {
                continue;
            }
            if obsolete.contains(&n) {
                old.push(d.clone());
                continue;
            }
            // Not listed, but another version of the same extension is: superseded copy.
            if let Some(l) = &listed {
                if !l.contains(&n) {
                    let id = n.rsplit_once('-').map(|(id, _)| id).unwrap_or(&n);
                    if l.iter().any(|x| x.rsplit_once('-').map(|(i, _)| i) == Some(id)) {
                        old.push(d.clone());
                    }
                }
            }
        }
        if old.is_empty() {
            continue;
        }
        let size = du_all(&old);
        if size.bytes < MIN / 4 {
            continue;
        }
        let names: Vec<String> = old.iter().map(|p| name_of(p)).collect();
        let busy = c.procs().app_named(app).map(|p| format!("{} is running", who(p)));
        out.push(
            make("versions.extensions", Category::OldVersions, format!("{} extensions", editor), "box", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, format!("{} superseded extension folders the editor marked obsolete or no longer lists; the versions it loads stay.", old.len()))
                .keyed(&s(&root))
                .detail(names.join(", "))
                .busy(busy),
        );
    }
    let _ = exists;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_match_prefixes() {
        assert!(pin_matches("20", &[20, 11, 1]));
        assert!(!pin_matches("20", &[21, 0, 0]));
        assert!(!pin_matches("lts/iron", &[20, 0, 0]));
        assert!(toolchain_pinned("1.93-aarch64-apple-darwin", &["1.93".into()]));
        assert!(!toolchain_pinned("1.93.1-aarch64-apple-darwin", &["1.93".into()]));
    }
}
