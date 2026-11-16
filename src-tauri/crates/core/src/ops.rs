//! The only ways Disko removes files. Everything (UI, routines, MCP) goes through here, so the
//! protected-path guard can't be skipped.

use crate::paths::is_protected;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Allocated bytes of a path (recursive, no symlink following, hard links counted once per call).
pub fn allocated(path: &Path) -> u64 {
    fn walk(p: &Path, seen: &mut std::collections::HashSet<(u64, u64)>) -> u64 {
        let Ok(m) = fs::symlink_metadata(p) else { return 0 };
        if m.nlink() > 1 && !m.is_dir() && !seen.insert((m.dev(), m.ino())) {
            return 0;
        }
        let mut total = m.blocks() * 512;
        if m.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    total += walk(&e.path(), seen);
                }
            }
        }
        total
    }
    walk(path, &mut Default::default())
}

fn guard(path: &Path) -> Result<(), String> {
    if is_protected(path) {
        return Err(format!("{} is protected; Disko never removes it.", path.display()));
    }
    if fs::symlink_metadata(path).is_err() {
        return Err("Already gone.".into());
    }
    Ok(())
}

/// Delete permanently. Symlinks are removed, never followed.
pub fn delete(path: &Path) -> Result<(), String> {
    guard(path)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let r = if m.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    r.map_err(|e| e.to_string())
}

/// Move to the user's Trash (Finder's "Put Back" works). Returns where it ended up.
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    guard(path)?;
    trash_native(path)
}

#[cfg(target_os = "macos")]
fn trash_native(path: &Path) -> Result<PathBuf, String> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let s = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&s);
    let fm = NSFileManager::defaultManager();
    let mut out: Option<Retained<NSURL>> = None;
    let res = fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut out));
    match res {
        Ok(()) => {
            let to = out.and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())).unwrap_or_default();
            Ok(to)
        }
        Err(e) => Err(e.localizedDescription().to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
fn trash_native(_path: &Path) -> Result<PathBuf, String> {
    Err("Trash is only supported on macOS".into())
}

/// Undo a trash: move `trashed` back to `original` if nothing took its place meanwhile.
pub fn put_back(trashed: &Path, original: &Path) -> Result<(), String> {
    if fs::symlink_metadata(original).is_ok() {
        return Err(format!("Something new is at {} already.", original.display()));
    }
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(trashed, original).map_err(|e| e.to_string())
}

/// Run a tool's own cleanup command. Returns combined output on failure.
pub fn run_command(argv: &[String]) -> Result<(), String> {
    let (prog, args) = argv.split_first().ok_or("Empty command")?;
    let out = std::process::Command::new(prog)
        .args(args)
        .env("PATH", format!("/opt/homebrew/bin:/usr/local/bin:{}/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin", crate::paths::home().display()))
        .output()
        .map_err(|e| format!("{}: {}", prog, e))?;
    if out.status.success() {
        Ok(())
    } else {
        let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if msg.is_empty() {
            msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        Err(msg.lines().last().unwrap_or("Command failed").to_string())
    }
}

/// Delete root-owned paths with one administrator prompt (macOS authorization dialog).
pub fn delete_as_admin(paths: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        if is_protected(p) {
            return Err(format!("{} is protected; Disko never removes it.", p.display()));
        }
    }
    let quoted: Vec<String> = paths.iter().map(|p| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))).collect();
    let shell = format!("/bin/rm -rf -- {}", quoted.join(" "));
    let script = format!("do shell script \"{}\" with administrator privileges", shell.replace('\\', "\\\\").replace('"', "\\\""));
    let out = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_protected() {
        assert!(delete(Path::new("/")).is_err());
        assert!(delete(&crate::paths::home()).is_err());
        assert!(delete(&crate::paths::home().join("Documents")).is_err());
        assert!(trash(Path::new("/System/Library")).is_err());
    }

    #[test]
    fn delete_and_trash_roundtrip() {
        let dir = std::env::temp_dir().join(format!("disko-core-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/f"), vec![1u8; 10_000]).unwrap();
        assert!(allocated(&dir) >= 10_000);
        delete(&dir.join("a")).unwrap();
        assert!(!dir.join("a").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_are_order_independent() {
        let a = crate::finding_id("r", &["/x".into(), "/y".into()]);
        let b = crate::finding_id("r", &["/y".into(), "/x".into()]);
        assert_eq!(a, b);
        assert!(a.starts_with("r:"));
    }
}
