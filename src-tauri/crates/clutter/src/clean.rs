//! Cleaning: carry out each finding's `Action` through `disko_core::ops` only.
//!
//! * `Delete` only for `Safety::Safe` (and the Trash itself); a Review finding is always moved to
//!   the Trash instead, whatever its action says.
//! * `Command` runs the owning tool once per distinct command line.
//! * Root-owned paths (`admin`) of Safe findings whose action is Delete are deleted together with
//!   one administrator prompt; any other root-owned path is skipped (never deleted instead of
//!   being moved to the Trash).
//! * Right before acting, every path is rechecked: keep list, in use (fresh process snapshot),
//!   still there, no symlinked ancestor. The guard in `ops` refuses protected paths.

use crate::procs::{who, Procs};
use crate::util::{du, s};
use crate::Clutter;
use disko_core::{ops, Action, CleanFailure, CleanResult, Category, Finding, Safety, Trashed};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// `progress(done, total, title)`, called before each finding.
pub type CleanProgress<'a> = &'a mut dyn FnMut(usize, usize, &str);

/// Process names that must not be running while a rule's paths are removed.
const RULE_TOOLS: &[(&str, &[&str])] = &[
    ("packages.npm", &["npm"]),
    ("packages.npx", &["npx"]),
    ("packages.pnpm", &["pnpm"]),
    ("packages.yarn", &["yarn"]),
    ("packages.bun", &["bun"]),
    ("packages.cargo", &["cargo", "rustc"]),
    ("packages.uv", &["uv"]),
    ("packages.pip", &["pip", "pip3"]),
    ("packages.go", &["go"]),
    ("packages.gradle", &["gradle", "java"]),
    ("packages.cocoapods", &["pod"]),
    ("packages.swiftpm", &["swift-build", "swift-package", "xcodebuild"]),
    ("packages.homebrew", &["brew"]),
    ("versions.homebrew", &["brew"]),
    ("leftovers.homebrew-deps", &["brew"]),
    ("developer.", &["xcodebuild", "XCBBuildService", "SWBBuildService"]),
    ("versions.rust-toolchain", &["cargo", "rustc", "rustup"]),
    ("extras.rust-docs", &["rustup"]),
];

const BUILD_TOOLS: &[&str] = &["cargo", "rustc", "rust-analyzer", "node", "npm", "pnpm", "yarn", "bun", "next-server", "vite", "swift-build", "swift-frontend", "xcodebuild", "python", "python3", "uv", "pytest", "gradle", "java", "pod"];

impl Clutter {
    /// Is this finding in use right now?
    pub fn busy_now(&self, f: &Finding, procs: &Procs) -> Option<String> {
        if !procs.complete {
            return Some("Couldn't check what's running".into());
        }
        for p in &f.paths {
            let p = Path::new(p);
            if let Some(x) = procs.cwd_inside(p).or_else(|| procs.exe_inside(p)) {
                return Some(format!("{} is using it", who(x)));
            }
            if f.category == Category::Build {
                if let Some(x) = p.parent().and_then(|proj| procs.tool_in(proj, BUILD_TOOLS)) {
                    return Some(format!("{} is running in this project", who(x)));
                }
            }
        }
        if let Some(app) = f.icon.strip_prefix("app:") {
            if !matches!(f.category, Category::Installers | Category::OldVersions) {
                if let Some(x) = procs.app_running(Path::new(app)) {
                    return Some(format!("{} is running", who(x)));
                }
            }
        }
        for (prefix, tools) in RULE_TOOLS {
            if f.rule.starts_with(prefix) {
                if let Some(x) = procs.named(tools) {
                    return Some(format!("{} is running", who(x)));
                }
            }
        }
        None
    }

    /// Remove what the findings describe. `is_kept(path)` must return true for a path that is on
    /// the keep list or contains something that is.
    ///
    /// Accounting:
    /// * `freed` counts only bytes actually released: permanent deletes (measured before
    ///   removing), administrator deletes that left nothing behind, and successful commands
    ///   (the finding's paths measured before and after; a command without paths counts its
    ///   estimate). Bytes moved to the Trash are not freed yet: they are in `trashed[].bytes`.
    /// * `deleted` lists every path removed permanently (by us, with the administrator password,
    ///   or by a successful command); `trashed` every path moved to the Trash.
    /// * Findings that share one command (e.g. `brew cleanup`) run it once and share its outcome:
    ///   if any of them is kept or busy, it runs for none of them; if it fails, all of them fail.
    /// * Root-owned paths are removed with the administrator password only when the finding is
    ///   Safe and its action is Delete; otherwise they are skipped, never deleted in place of the
    ///   Trash.
    pub fn clean(&self, findings: &[Finding], is_kept: &dyn Fn(&str) -> bool, progress: CleanProgress) -> CleanResult {
        let procs = self.fresh_procs();
        let mut res = CleanResult::default();
        // Every finding that shares a command line, by command.
        let mut groups: HashMap<&[String], Vec<&Finding>> = HashMap::new();
        for f in findings {
            if let Action::Command { argv, .. } = &f.action {
                groups.entry(argv.as_slice()).or_default().push(f);
            }
        }
        let mut commands_done: HashSet<Vec<String>> = HashSet::new();
        let mut admin_batch: Vec<(String, PathBuf, u64)> = vec![]; // (finding id, path, bytes)
        let total = findings.len();
        for (i, f) in findings.iter().enumerate() {
            progress(i, total, &f.title);
            if let Action::Command { argv, label } = &f.action {
                if commands_done.insert(argv.clone()) {
                    let group = groups.get(argv.as_slice()).cloned().unwrap_or_else(|| vec![f]);
                    self.run_shared_command(&group, argv, label, &procs, is_kept, &mut res);
                }
                continue;
            }
            if let Some(why) = self.busy_now(f, &procs) {
                skip_all(f, &why, &mut res);
                continue;
            }
            let trash = matches!(f.action, Action::Trash) || (f.safety == Safety::Review && f.category != Category::Trash);
            let mut all_done = true;
            for p in &f.paths {
                let path = PathBuf::from(p);
                if is_kept(p) {
                    res.skipped.push(CleanFailure { path: p.clone(), error: "On your keep list".into() });
                    all_done = false;
                    continue;
                }
                if let Err(e) = check_path(&path) {
                    if e != "Already gone." {
                        res.failed.push(CleanFailure { path: p.clone(), error: e });
                        all_done = false;
                    }
                    continue;
                }
                let bytes = du(&path).bytes;
                if f.admin && crate::util::needs_admin(&path) {
                    if !trash && f.safety == Safety::Safe && matches!(f.action, Action::Delete) {
                        admin_batch.push((f.id.clone(), path, bytes));
                    } else {
                        res.skipped.push(CleanFailure { path: p.clone(), error: NEEDS_ADMIN.into() });
                        all_done = false;
                    }
                    continue;
                }
                let r = if trash { ops::trash(&path).map(|to| res.trashed.push(Trashed { from: p.clone(), to: s(&to), bytes })) } else { delete_writable(&path) };
                match r {
                    Ok(()) if trash => {}
                    Ok(()) => {
                        res.freed += bytes;
                        res.deleted.push(p.clone());
                    }
                    Err(e) => {
                        res.failed.push(CleanFailure { path: p.clone(), error: e });
                        all_done = false;
                    }
                }
            }
            let pending_admin = admin_batch.iter().any(|(id, _, _)| id == &f.id);
            if all_done && !pending_admin {
                res.removed.push(f.id.clone());
            }
        }
        if !admin_batch.is_empty() {
            progress(total, total, "Administrator items");
            let paths: Vec<PathBuf> = admin_batch.iter().map(|x| x.1.clone()).collect();
            match ops::delete_as_admin(&paths) {
                Ok(()) => {
                    let mut ids: Vec<&String> = vec![];
                    for (id, p, bytes) in &admin_batch {
                        if fs::symlink_metadata(p).is_err() {
                            res.freed += bytes;
                            res.deleted.push(s(p));
                        } else {
                            res.failed.push(CleanFailure { path: s(p), error: "Still there after removing".into() });
                        }
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    for id in ids {
                        let failed = admin_batch.iter().any(|(i, p, _)| i == id && fs::symlink_metadata(p).is_ok());
                        let other = findings.iter().filter(|f| &f.id == id).any(|f| res.failed.iter().chain(res.skipped.iter()).any(|x| f.paths.contains(&x.path)));
                        if !failed && !other {
                            res.removed.push(id.clone());
                        }
                    }
                }
                Err(e) => {
                    for (_, p, _) in &admin_batch {
                        res.failed.push(CleanFailure { path: s(p), error: if e.is_empty() { "Administrator access was declined".into() } else { e.clone() } });
                    }
                }
            }
        }
        progress(total, total, "Done");
        res
    }

    /// Run one command shared by `group` (every finding with that command line), once, and give
    /// all of them its outcome.
    fn run_shared_command(&self, group: &[&Finding], argv: &[String], label: &str, procs: &Procs, is_kept: &dyn Fn(&str) -> bool, res: &mut CleanResult) {
        // The command removes every sharing finding's paths, so any keep or busy blocks all.
        let kept = group.iter().flat_map(|f| f.paths.iter()).find(|p| is_kept(p)).cloned();
        let busy = group.iter().find_map(|f| self.busy_now(f, procs));
        let blocked = match (kept, busy) {
            (Some(k), _) => Some((Some(k), format!("`{}` would also remove something on your keep list", label))),
            (None, Some(why)) => Some((None, why)),
            (None, None) => None,
        };
        if let Some((kept_path, why)) = blocked {
            for f in group {
                match &kept_path {
                    Some(k) if f.paths.contains(k) => res.skipped.push(CleanFailure { path: k.clone(), error: "On your keep list".into() }),
                    _ => skip_all(f, &why, res),
                }
            }
            return;
        }
        let mut paths: Vec<&String> = group.iter().flat_map(|f| f.paths.iter()).filter(|p| fs::symlink_metadata(p).is_ok()).collect();
        paths.sort();
        paths.dedup();
        let before: Vec<u64> = paths.iter().map(|p| du(Path::new(p)).bytes).collect();
        match ops::run_command(argv) {
            Ok(()) => {
                for (p, b) in paths.iter().zip(&before) {
                    if fs::symlink_metadata(p).is_err() {
                        res.freed += b;
                        res.deleted.push((*p).clone());
                    } else {
                        res.freed += b.saturating_sub(du(Path::new(p)).bytes);
                    }
                }
                for f in group {
                    if f.paths.is_empty() {
                        res.freed += f.bytes;
                    }
                    res.removed.push(f.id.clone());
                }
            }
            Err(e) => {
                for f in group {
                    if f.paths.is_empty() {
                        res.failed.push(CleanFailure { path: label.to_string(), error: e.clone() });
                    }
                    for p in &f.paths {
                        res.failed.push(CleanFailure { path: p.clone(), error: format!("{} failed: {}", label, e) });
                    }
                }
            }
        }
    }
}

/// Shown for a root-owned item Disko won't remove permanently on its own.
pub const NEEDS_ADMIN: &str = "Needs your administrator password to remove permanently — remove it in Finder or with its uninstaller.";

/// Skip every path of a finding (or its title when it has none) for `why`.
fn skip_all(f: &Finding, why: &str, res: &mut CleanResult) {
    for p in &f.paths {
        res.skipped.push(CleanFailure { path: p.clone(), error: why.to_string() });
    }
    if f.paths.is_empty() {
        res.skipped.push(CleanFailure { path: f.title.clone(), error: why.to_string() });
    }
}

/// Still there, absolute, no `..`, and no symlink on the way to it (the path itself may be a
/// link: removing a link never touches its target).
pub fn check_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("Not an absolute path".into());
    }
    if fs::symlink_metadata(path).is_err() {
        return Err("Already gone.".into());
    }
    let parent = path.parent().ok_or("No parent")?;
    let real = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if real != parent {
        return Err("Its folder is reached through a link; skipped".into());
    }
    Ok(())
}

/// Delete through `ops`; if read-only files block it (Go modules, some build output), make our
/// own files writable and try once more.
fn delete_writable(path: &Path) -> Result<(), String> {
    match ops::delete(path) {
        Ok(()) => Ok(()),
        Err(e) if fs::symlink_metadata(path).is_ok() => {
            make_writable(path);
            ops::delete(path).map_err(|e2| if e2.is_empty() { e } else { e2 })
        }
        Err(e) => Err(e),
    }
}

fn make_writable(path: &Path) {
    let Ok(m) = fs::symlink_metadata(path) else { return };
    if m.file_type().is_symlink() {
        return;
    }
    if m.is_dir() {
        let mut perm = m.permissions();
        perm.set_mode(perm.mode() | 0o700);
        let _ = fs::set_permissions(path, perm);
        for e in fs::read_dir(path).into_iter().flatten().flatten() {
            make_writable(&e.path());
        }
    }
}
