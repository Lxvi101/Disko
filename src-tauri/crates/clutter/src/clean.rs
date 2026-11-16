//! Cleaning: carry out each finding's `Action` through `disko_core::ops` only.
//!
//! * `Delete` only for `Safety::Safe` (and the Trash itself); a Review finding is always moved to
//!   the Trash instead, whatever its action says.
//! * `Command` runs the owning tool once per distinct command line.
//! * Root-owned paths (`admin`) are deleted together with one administrator prompt.
//! * Right before acting, every path is rechecked: keep list, in use (fresh process snapshot),
//!   still there, no symlinked ancestor. The guard in `ops` refuses protected paths.

use crate::procs::{who, Procs};
use crate::util::{du, s};
use crate::Clutter;
use disko_core::{ops, Action, CleanFailure, CleanResult, Category, Finding, Safety, Trashed};
use std::collections::HashSet;
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
    pub fn clean(&self, findings: &[Finding], is_kept: &dyn Fn(&str) -> bool, progress: CleanProgress) -> CleanResult {
        let procs = self.fresh_procs();
        let mut res = CleanResult::default();
        let mut commands_run: HashSet<Vec<String>> = HashSet::new();
        let mut admin_batch: Vec<(String, PathBuf, u64)> = vec![]; // (finding id, path, bytes)
        let total = findings.len();
        for (i, f) in findings.iter().enumerate() {
            progress(i, total, &f.title);
            if let Some(why) = self.busy_now(f, &procs) {
                for p in &f.paths {
                    res.skipped.push(CleanFailure { path: p.clone(), error: why.clone() });
                }
                if f.paths.is_empty() {
                    res.skipped.push(CleanFailure { path: f.title.clone(), error: why });
                }
                continue;
            }
            match &f.action {
                Action::Command { argv, label } => {
                    if f.paths.iter().any(|p| is_kept(p)) {
                        res.skipped.push(CleanFailure { path: f.paths.iter().find(|p| is_kept(p)).cloned().unwrap_or_default(), error: "On your keep list".into() });
                        continue;
                    }
                    let existing: Vec<&String> = f.paths.iter().filter(|p| fs::symlink_metadata(p).is_ok()).collect();
                    let before: u64 = existing.iter().map(|p| du(Path::new(p)).bytes).sum();
                    if !commands_run.insert(argv.clone()) {
                        // Same command already ran for another finding (e.g. brew cleanup).
                        let after: u64 = existing.iter().map(|p| du(Path::new(p)).bytes).sum();
                        res.freed += before.saturating_sub(after);
                        res.removed.push(f.id.clone());
                        continue;
                    }
                    match ops::run_command(argv) {
                        Ok(()) => {
                            let freed = if f.paths.is_empty() { f.bytes } else { before.saturating_sub(existing.iter().map(|p| du(Path::new(p)).bytes).sum()) };
                            res.freed += freed;
                            res.removed.push(f.id.clone());
                        }
                        Err(e) => res.failed.push(CleanFailure { path: label.clone(), error: e }),
                    }
                }
                action => {
                    let trash = matches!(action, Action::Trash) || (f.safety == Safety::Review && f.category != Category::Trash);
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
                            admin_batch.push((f.id.clone(), path, bytes));
                            continue;
                        }
                        let r = if trash { ops::trash(&path).map(|to| res.trashed.push(Trashed { from: p.clone(), to: s(&to), bytes })) } else { delete_writable(&path) };
                        match r {
                            Ok(()) => res.freed += if trash { 0 } else { bytes },
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
                        } else {
                            res.failed.push(CleanFailure { path: s(p), error: "Still there after removing".into() });
                        }
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    for id in ids {
                        let failed = admin_batch.iter().any(|(i, p, _)| i == id && fs::symlink_metadata(p).is_ok());
                        if !failed && !res.failed.iter().any(|x| findings.iter().any(|f| &f.id == id && f.paths.contains(&x.path))) {
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
