//! Paths Disko itself uses, and guards against touching what it must never touch.

use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/Library/Application Support/app.disko` (memory, settings, community pack). Created on demand.
pub fn support_dir() -> PathBuf {
    let d = home().join("Library/Application Support/app.disko");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// `~/Library/Caches/app.disko` (the one disk snapshot). Safe for anyone to delete.
pub fn cache_dir() -> PathBuf {
    let d = home().join("Library/Caches/app.disko");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// "~/x/y" for display.
pub fn short(path: &str) -> String {
    let h = home();
    let h = h.to_string_lossy();
    match path.strip_prefix(h.as_ref()) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{}", rest),
        _ => path.to_string(),
    }
}

/// Lower-cased string form, for case-insensitive comparisons (APFS is case-insensitive).
fn lc(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn inside(p: &str, root: &str) -> bool {
    p == root || p.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Paths that no clean, rule or agent may ever remove, whatever they say. Checked by `ops`
/// against both the path as given and its resolved form. Case-insensitive.
pub fn is_protected(path: &Path) -> bool {
    use std::path::Component;
    let h = home();
    if !path.is_absolute() || path.components().count() < 3 {
        return true; // "/", "/Users", relative paths
    }
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return true; // "~/Desktop/../.ssh" style paths are never accepted
    }
    let p = lc(path);
    let hh = |rel: &str| lc(&h.join(rel));
    let exact_or_inside = [
        "/system".to_string(),
        "/usr/bin".into(),
        "/usr/lib".into(),
        "/usr/libexec".into(),
        "/usr/sbin".into(),
        "/usr/share".into(),
        "/bin".into(),
        "/sbin".into(),
        "/etc".into(),
        "/private/etc".into(),
        "/volumes".into(),
        "/library/keychains".into(),
        hh("Library/Keychains"),
        hh("Library/Mobile Documents"),
        hh("Library/CloudStorage"),
        hh("Library/Mail"),
        hh("Library/Messages"),
        hh("Library/Photos"),
        hh("Library/Safari"),
        hh("Library/Calendars"),
        hh("Library/Application Support/AddressBook"),
        hh("Library/Group Containers/group.com.apple.notes"),
        hh("Library/Accounts"),
        hh(".ssh"),
        hh(".gnupg"),
        hh(".aws"),
        hh(".kube"),
        hh(".config/gh"),
        hh(".docker/config.json"),
        hh(".password-store"),
    ];
    if exact_or_inside.iter().any(|x| inside(&p, x)) {
        return true;
    }
    // Whole top-level user folders and the home itself.
    let exact = [
        lc(&h),
        hh("Library"),
        hh("Documents"),
        hh("Desktop"),
        hh("Downloads"),
        hh("Pictures"),
        hh("Movies"),
        hh("Music"),
        hh("Applications"),
        hh("Developer"),
        hh("Development"),
        hh("Library/Application Support"),
        hh("Library/Caches"),
        hh("Library/Containers"),
        hh("Library/Group Containers"),
        hh("Library/Preferences"),
        hh("Library/Developer"),
        hh(".Trash"),
        hh(".config"),
        hh(".local"),
        hh(".cache"),
        "/applications".into(),
        "/library".into(),
        "/library/application support".into(),
        "/library/caches".into(),
        "/private".into(),
        "/private/var".into(),
        "/private/tmp".into(),
        "/opt".into(),
        "/opt/homebrew".into(),
        "/usr/local".into(),
        "/users".into(),
    ];
    if exact.iter().any(|x| &p == x) {
        return true;
    }
    // Repositories' history and photo libraries, wherever they are.
    p.ends_with(".photoslibrary") || p.contains(".photoslibrary/") || p.ends_with("/.git") || p.contains("/.git/")
}

/// `path` with its parent folder resolved (symlinks, case), or None when the parent is gone.
/// The last component itself is not followed, so a symlink is judged as the link.
pub fn resolved(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    Some(std::fs::canonicalize(parent).ok()?.join(name))
}
