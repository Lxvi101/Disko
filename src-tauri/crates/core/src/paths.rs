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

/// Paths that no clean, rule or agent may ever remove, whatever they say. Checked by `ops`.
/// (Exact paths, or anything inside them.)
pub fn is_protected(path: &Path) -> bool {
    let h = home();
    let p = path;
    if !p.is_absolute() || p.components().count() < 3 {
        return true; // "/", "/Users", relative paths
    }
    let exact_or_inside = [
        PathBuf::from("/System"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/libexec"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        PathBuf::from("/private/etc"),
        PathBuf::from("/Library/Keychains"),
        h.join("Library/Keychains"),
        h.join("Library/Mobile Documents"),
        h.join("Library/CloudStorage"),
        h.join("Library/Mail"),
        h.join("Library/Messages"),
        h.join("Library/Photos"),
        h.join(".ssh"),
        h.join(".gnupg"),
    ];
    if exact_or_inside.iter().any(|x| p.starts_with(x)) {
        return true;
    }
    // Whole top-level user folders and the home itself.
    let exact = [
        h.clone(),
        h.join("Library"),
        h.join("Documents"),
        h.join("Desktop"),
        h.join("Downloads"),
        h.join("Pictures"),
        h.join("Movies"),
        h.join("Music"),
        h.join("Applications"),
        h.join("Library/Application Support"),
        h.join("Library/Caches"),
        h.join("Library/Containers"),
        h.join("Library/Group Containers"),
        h.join("Library/Preferences"),
        h.join(".Trash"),
        PathBuf::from("/Applications"),
        PathBuf::from("/Library"),
        PathBuf::from("/Library/Application Support"),
        PathBuf::from("/Library/Caches"),
        PathBuf::from("/private/var"),
        PathBuf::from("/opt/homebrew"),
        PathBuf::from("/usr/local"),
    ];
    if exact.iter().any(|x| p == x) {
        return true;
    }
    let s = p.to_string_lossy();
    s.ends_with(".photoslibrary") || s.ends_with("/.git") || s.contains(".photoslibrary/")
}
