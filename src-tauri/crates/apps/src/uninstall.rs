//! Uninstall: the app and everything only it owns go to the Trash (Undo puts them back).
//!
//! Refuses apps that ship with macOS, apps that are running (skipped as busy), and apps whose
//! vendor requires its own uninstaller (Adobe: licensing, shared components and helpers are
//! tracked by Creative Cloud; trashing the bundle leaves those behind and confuses CC).
//! Root-owned items that the Trash can't take are deleted with one administrator prompt for the
//! whole batch, only from this explicit user action, and reported in `deleted` (no Undo).

use crate::apps::{inventory, Env};
use crate::size;
use crate::types::{AppInfo, Uninstall};
use crate::util::s;
use disko_core::{CleanFailure, Trashed};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// How removal happens; the real one uses `disko_core::ops`, tests use a temp "Trash".
pub trait Remover {
    fn trash(&self, path: &Path) -> Result<PathBuf, String>;
    fn delete_as_admin(&self, paths: &[PathBuf]) -> Result<(), String>;
}

pub struct CoreOps;

impl Remover for CoreOps {
    fn trash(&self, path: &Path) -> Result<PathBuf, String> {
        disko_core::ops::trash(path)
    }
    fn delete_as_admin(&self, paths: &[PathBuf]) -> Result<(), String> {
        disko_core::ops::delete_as_admin(paths)
    }
}

/// Uninstall these app bundles (paths as listed by `apps()`).
pub fn uninstall(app_paths: &[String]) -> Uninstall {
    uninstall_with(&Env::real(), app_paths, &CoreOps, true)
}

fn needs_admin(path: &Path) -> bool {
    let Ok(m) = fs::symlink_metadata(path) else { return false };
    if m.uid() == 0 {
        return true;
    }
    // The Trash needs write access to the parent folder.
    path.parent().is_some_and(|p| {
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default();
        // SAFETY: access(2) with a valid C string.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) != 0 }
    })
}

fn vendor_note(app: &AppInfo) -> String {
    let vendor = if app.bundle_id.to_ascii_lowercase().starts_with("com.adobe.") { "Adobe" } else { "Its maker" };
    match &app.uninstaller {
        Some(u) => format!("{} has its own uninstaller for {}: open {}.", vendor, app.name, u),
        None => format!("{} must be removed with {}'s own uninstaller.", app.name, vendor),
    }
}

pub fn uninstall_with(env: &Env, app_paths: &[String], remover: &dyn Remover, allow_admin: bool) -> Uninstall {
    let mut out = Uninstall::default();
    let list = inventory(env);
    let mut admin_queue: Vec<(PathBuf, u64, String)> = Vec::new(); // (path, bytes, app path)
    let mut app_failed: std::collections::HashSet<String> = Default::default();

    for wanted in app_paths {
        let Some(app) = list.iter().find(|a| &a.path == wanted || a.bundle_root.as_deref() == Some(wanted.as_str())) else {
            out.result.failed.push(CleanFailure { path: wanted.clone(), error: "Not an installed app Disko knows.".into() });
            continue;
        };
        if app.apple {
            out.result.failed.push(CleanFailure { path: app.path.clone(), error: format!("{} is part of macOS.", app.name) });
            continue;
        }
        let adobe = app.bundle_id.to_ascii_lowercase().starts_with("com.adobe.");
        if adobe || app.uninstaller.is_some() {
            let note = vendor_note(app);
            out.result.failed.push(CleanFailure { path: app.path.clone(), error: note.clone() });
            out.notes.push(note);
            continue;
        }
        if app.running {
            out.result.skipped.push(CleanFailure { path: app.path.clone(), error: format!("{} is running. Quit it first.", app.name) });
            continue;
        }
        let bundle = PathBuf::from(app.bundle_root.as_deref().unwrap_or(&app.path));
        let mut items: Vec<PathBuf> = vec![bundle.clone()];
        items.extend(app.data_paths.iter().map(PathBuf::from));
        for (i, p) in items.iter().enumerate() {
            if fs::symlink_metadata(p).is_err() {
                continue;
            }
            let bytes = size::bytes(p);
            match remover.trash(p) {
                Ok(to) => {
                    // Trash frees nothing yet; bytes are reported in `trashed`.
                    out.result.trashed.push(Trashed { from: s(p), to: s(&to), bytes });
                }
                // The app itself needs a password: ask now, and touch its data only if it's gone.
                Err(_) if i == 0 && needs_admin(p) => {
                    let res = if allow_admin { remover.delete_as_admin(std::slice::from_ref(p)) } else { Err("Needs an administrator password.".into()) };
                    match res {
                        Ok(()) => {
                            out.result.freed += bytes;
                            out.deleted.push(s(p));
                            out.notes.push(format!("{} was owned by the system, so it was deleted with your administrator password; it can't be put back.", app.name));
                        }
                        Err(e) => {
                            out.result.failed.push(CleanFailure { path: s(p), error: e });
                            app_failed.insert(app.path.clone());
                            break;
                        }
                    }
                }
                Err(_) if needs_admin(p) => admin_queue.push((p.clone(), bytes, app.path.clone())),
                Err(e) => {
                    out.result.failed.push(CleanFailure { path: s(p), error: e });
                    app_failed.insert(app.path.clone());
                    // Bundle couldn't go: leave its data alone so the app keeps working.
                    if i == 0 {
                        break;
                    }
                }
            }
        }
        size::invalidate(Some(&bundle));
        for d in &app.data_paths {
            size::invalidate(Some(Path::new(d)));
        }
        if !app_failed.contains(&app.path) {
            out.result.removed.push(app.path.clone());
        }
    }

    if !admin_queue.is_empty() {
        let paths: Vec<PathBuf> = admin_queue.iter().map(|(p, _, _)| p.clone()).collect();
        let result = if allow_admin { remover.delete_as_admin(&paths) } else { Err("Needs an administrator password.".into()) };
        match result {
            Ok(()) => {
                for (p, b, _) in &admin_queue {
                    out.result.freed += b;
                    out.deleted.push(s(p));
                }
                out.notes.push(format!("{} data item(s) owned by the system were deleted with your administrator password; they can't be put back.", admin_queue.len()));
            }
            Err(e) => {
                // The apps themselves are gone; only some root-owned data stays behind.
                for (p, _, _) in &admin_queue {
                    out.result.failed.push(CleanFailure { path: s(p), error: e.clone() });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_dir;
    use std::cell::RefCell;

    /// Moves into a temp "Trash" folder; records admin calls.
    struct FakeTrash {
        bin: PathBuf,
        admin: RefCell<Vec<PathBuf>>,
    }
    impl Remover for FakeTrash {
        fn trash(&self, path: &Path) -> Result<PathBuf, String> {
            if disko_core::paths::is_protected(path) {
                return Err("protected".into());
            }
            let to = self.bin.join(format!("{}-{}", self.bin.read_dir().unwrap().count(), path.file_name().unwrap().to_string_lossy()));
            fs::rename(path, &to).map_err(|e| e.to_string())?;
            Ok(to)
        }
        fn delete_as_admin(&self, paths: &[PathBuf]) -> Result<(), String> {
            self.admin.borrow_mut().extend(paths.iter().cloned());
            Ok(())
        }
    }

    #[test]
    fn trashes_app_and_its_data_but_not_shared_and_refuses_vendor_apps() {
        let home = test_dir("uninstall");
        let apps = home.join("Applications");
        let foo = crate::apps::tests::make_app(&apps, "Foo", "com.acme.Foo");
        crate::apps::tests::make_app(&apps, "Bar", "com.acme.Bar");
        let adobe = apps.join("Adobe Thing 2026");
        crate::apps::tests::make_app(&adobe, "Adobe Thing 2026", "com.adobe.Thing");
        let lib = home.join("Library");
        for p in ["Caches/com.acme.Foo/c", "Application Support/Foo/s", "Application Support/Acme/shared"] {
            fs::create_dir_all(lib.join(p).parent().unwrap()).unwrap();
            fs::write(lib.join(p), b"x").unwrap();
        }
        let bin = home.join("FakeTrash");
        fs::create_dir_all(&bin).unwrap();
        let fake = FakeTrash { bin, admin: RefCell::new(vec![]) };
        let env = Env::fixture(&home);

        let r = uninstall_with(&env, &[s(&foo), s(&adobe.join("Adobe Thing 2026.app")), s(&home.join("nope.app"))], &fake, false);
        assert_eq!(r.result.removed, vec![s(&foo)]);
        assert!(!foo.exists());
        assert!(!lib.join("Caches/com.acme.Foo").exists());
        assert!(!lib.join("Application Support/Foo").exists());
        // "Acme" vendor folder is claimed by Foo and Bar alike: shared, kept.
        assert!(lib.join("Application Support/Acme").exists());
        assert_eq!(r.result.trashed.len(), 3);
        assert_eq!(r.result.freed, 0, "Trash frees nothing yet");
        assert!(r.result.trashed.iter().map(|t| t.bytes).sum::<u64>() > 0);
        // Adobe app refused with a pointer to Adobe's uninstaller; unknown path fails.
        assert!(adobe.join("Adobe Thing 2026.app").exists());
        assert!(r.notes.iter().any(|n| n.contains("Adobe")));
        assert_eq!(r.result.failed.len(), 2);
        assert!(fake.admin.borrow().is_empty());
        // Undo puts everything back where it was.
        for t in &r.result.trashed {
            disko_core::ops::put_back(Path::new(&t.to), Path::new(&t.from)).unwrap();
        }
        assert!(foo.exists() && lib.join("Caches/com.acme.Foo/c").exists());
        fs::remove_dir_all(&home).unwrap();
    }
}
