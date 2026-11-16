//! Small read-only helpers shared by rules, probes and cleaning.

use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;
pub const DAY: i64 = 86_400;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn mtime(path: &Path) -> Option<i64> {
    fs::symlink_metadata(path).ok().map(|m| m.mtime())
}

pub fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn s(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// A real directory (not a symlink to one).
pub fn is_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// A real regular file (not a symlink).
pub fn is_file(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Entries of a directory, sorted, skipping `.DS_Store`.
pub fn children(path: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != ".DS_Store")
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

pub fn child_dirs(path: &Path) -> Vec<PathBuf> {
    children(path).into_iter().filter(|p| is_dir(p)).collect()
}

/// Allocated size of a tree.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    pub newest: Option<i64>,
}

impl Size {
    pub fn add(&mut self, o: Size) {
        self.bytes += o.bytes;
        self.files += o.files;
        self.newest = self.newest.max(o.newest);
    }
}

/// Allocated bytes, file count and newest mtime of a tree, on all cores. Hard links count once,
/// symlinks are never followed, other volumes are skipped.
pub fn du(path: &Path) -> Size {
    du_filtered(path, false)
}

/// Like [`du`], but counts only files that have a single link: what deleting a hard-linked
/// store (pnpm) would actually give back.
pub fn du_unlinked(path: &Path) -> Size {
    du_filtered(path, true)
}

fn du_filtered(path: &Path, only_single_link: bool) -> Size {
    fn walk(p: &Path, dev: u64, seen: &Mutex<HashSet<(u64, u64)>>, single: bool) -> Size {
        let Ok(m) = fs::symlink_metadata(p) else { return Size::default() };
        if m.dev() != dev {
            return Size::default();
        }
        if !m.is_dir() {
            if m.nlink() > 1 {
                if single {
                    return Size::default();
                }
                if !seen.lock().map(|mut s| s.insert((m.dev(), m.ino()))).unwrap_or(true) {
                    return Size::default();
                }
            }
            return Size { bytes: m.blocks() * 512, files: 1, newest: Some(m.mtime()) };
        }
        let mut own = Size { bytes: if single { 0 } else { m.blocks() * 512 }, files: 0, newest: Some(m.mtime()) };
        let kids: Vec<PathBuf> = fs::read_dir(p).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()).collect();
        let below = if kids.len() > 8 {
            kids.par_iter().map(|k| walk(k, dev, seen, single)).reduce(Size::default, |mut a, b| {
                a.add(b);
                a
            })
        } else {
            let mut acc = Size::default();
            for k in &kids {
                acc.add(walk(k, dev, seen, single));
            }
            acc
        };
        own.add(below);
        own
    }
    let Ok(m) = fs::symlink_metadata(path) else { return Size::default() };
    if m.file_type().is_symlink() {
        return Size { bytes: 0, files: 1, newest: Some(m.mtime()) };
    }
    walk(path, m.dev(), &Mutex::new(HashSet::new()), only_single_link)
}

/// Sum of [`du`] over several paths.
pub fn du_all<P: AsRef<Path> + Sync>(paths: &[P]) -> Size {
    paths.par_iter().map(|p| du(p.as_ref())).reduce(Size::default, |mut a, b| {
        a.add(b);
        a
    })
}

/// Newest mtime of a folder and its direct children (cheap activity check).
pub fn shallow_newest(path: &Path) -> Option<i64> {
    let mut n = mtime(path);
    for c in children(path).into_iter().take(500) {
        n = n.max(mtime(&c));
    }
    n
}

/// A PATH that finds the tools a GUI app doesn't see.
pub fn tool_path_env(home: &Path) -> String {
    format!(
        "/opt/homebrew/bin:/usr/local/bin:{h}/.cargo/bin:{h}/.local/bin:{h}/.bun/bin:{h}/Library/pnpm:{h}/Library/pnpm/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        h = home.display()
    )
}

/// Where a command-line tool lives, if installed.
pub fn tool(home: &Path, bin: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&tool_path_env(home)).collect();
    dirs.push("/Applications/OrbStack.app/Contents/MacOS/xbin".into());
    dirs.push("/Applications/Docker.app/Contents/Resources/bin".into());
    dirs.push(home.join(".orbstack/bin"));
    dirs.push(home.join(".volta/bin"));
    dirs.push(home.join(".local/share/pnpm"));
    dirs.into_iter().map(|d| d.join(bin)).find(|p| p.is_file())
}

/// Run a read-only command with a time limit. Returns stdout when it exits successfully.
pub fn run(prog: &Path, args: &[&str], home: &Path, timeout: Duration) -> Option<String> {
    let mut child = Command::new(prog)
        .args(args)
        .env("PATH", tool_path_env(home))
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = (&mut out).take(32 * MB).read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(_) => break None,
        }
    };
    let bytes = reader.join().ok()?;
    status.filter(|s| s.success())?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// Numeric version from a name: "v20.11.1" → [20,11,1], "2.1.325" → [2,1,325],
/// "0.169.0-aarch64-apple-darwin" → [0,169,0], "IntelliJIdea2025.1" → None (use [`trailing_version`]).
pub fn parse_version(name: &str) -> Option<Vec<u64>> {
    let t = name.strip_prefix('v').unwrap_or(name);
    let head: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let head = head.trim_end_matches('.');
    if head.is_empty() || !head.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let rest = &t[head.len()..];
    if !(rest.is_empty() || rest.starts_with(['-', '_', '+', ' ', '(']) ) {
        return None;
    }
    head.split('.').map(|p| p.parse().ok()).collect()
}

/// Product + version from names like "IntelliJIdea2025.1" or "PyCharm2024.3".
pub fn trailing_version(name: &str) -> Option<(String, Vec<u64>)> {
    let idx = name.find(|c: char| c.is_ascii_digit())?;
    let (prod, ver) = name.split_at(idx);
    if prod.is_empty() {
        return None;
    }
    Some((prod.to_string(), parse_version(ver)?))
}

pub fn version_str(v: &[u64]) -> String {
    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".")
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= GB as f64 {
        format!("{:.1} GB", b / GB as f64)
    } else if b >= MB as f64 {
        format!("{:.0} MB", b / MB as f64)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

/// "today", "3 days ago", "4 months ago", "2 years ago".
pub fn ago(t: Option<i64>) -> String {
    let Some(t) = t else { return "at an unknown time".into() };
    let d = (now() - t).max(0) / DAY;
    match d {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=44 => format!("{} days ago", d),
        45..=544 => format!("{} months ago", (d + 15) / 30),
        _ => format!("{} years ago", (d + 182) / 365),
    }
}

/// A readable name from a bundle id or folder name: "com.tinyspeck.slackmacgap" → "Slack".
pub fn pretty_bundle(raw: &str) -> String {
    let known = [
        ("slackmacgap", "Slack"),
        ("spotify", "Spotify"),
        ("hnc", "Discord"),
        ("todesktop", "Desktop app"),
        ("vscode", "VS Code"),
        ("googlechrome", "Google Chrome"),
        ("Chrome", "Google Chrome"),
        ("thebrowser", "Arc"),
        ("anthropic", "Claude"),
        ("openai", "ChatGPT"),
    ];
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() >= 3 && ["com", "net", "org", "io", "app", "dev", "co", "us", "me", "ai", "de", "fr", "company", "so", "sh", "tv"].contains(&parts[0]) {
        for (k, v) in known {
            if parts[1..].iter().any(|p| p.eq_ignore_ascii_case(k)) {
                return v.into();
            }
        }
        let generic = ["client", "app", "desktop", "mac", "macos", "helper", "main", "electron", "osx"];
        let pick = if generic.contains(&parts[2].to_lowercase().as_str()) { parts[1].to_string() } else { parts[2..].join(" ") };
        let pick = cap(&pick.replace(['-', '_'], " "));
        // "com.ableton.live" → "Ableton Live": a short product name reads better with its maker.
        if parts.len() == 3 && pick.len() <= 4 && !generic.contains(&parts[2].to_lowercase().as_str()) {
            return format!("{} {}", cap(parts[1]), pick);
        }
        return pick;
    }
    cap(raw)
}

pub fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Root-owned or not writable by us: removing needs an administrator.
pub fn needs_admin(path: &Path) -> bool {
    let uid = unsafe { libc::getuid() };
    let owned_by_other = fs::symlink_metadata(path).map(|m| m.uid() != uid).unwrap_or(false);
    let parent_locked = path
        .parent()
        .map(|p| {
            let c = std::ffi::CString::new(p.as_os_str().to_string_lossy().as_bytes()).unwrap_or_default();
            unsafe { libc::access(c.as_ptr(), libc::W_OK) != 0 }
        })
        .unwrap_or(true);
    owned_by_other || parent_locked
}

/// String values from a plist dictionary.
pub fn plist_dict(path: &Path) -> Option<plist::Dictionary> {
    plist::Value::from_file(path).ok()?.into_dictionary()
}

pub fn plist_str(d: &plist::Dictionary, key: &str) -> Option<String> {
    d.get(key).and_then(|v| v.as_string()).map(|s| s.to_string())
}

/// Path relative to home, if inside it.
pub fn rel<'a>(path: &'a Path, home: &Path) -> Option<&'a Path> {
    path.strip_prefix(home).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse() {
        assert_eq!(parse_version("v20.11.1"), Some(vec![20, 11, 1]));
        assert_eq!(parse_version("0.169.0-aarch64-apple-darwin"), Some(vec![0, 169, 0]));
        assert_eq!(parse_version("2.1.325"), Some(vec![2, 1, 325]));
        assert_eq!(parse_version("stable-aarch64"), None);
        assert_eq!(parse_version("1.2abc"), None);
        assert_eq!(trailing_version("IntelliJIdea2025.1"), Some(("IntelliJIdea".into(), vec![2025, 1])));
    }

    #[test]
    fn names_read_well() {
        assert_eq!(pretty_bundle("com.spotify.client"), "Spotify");
        assert_eq!(pretty_bundle("com.tinyspeck.slackmacgap"), "Slack");
        assert_eq!(pretty_bundle("com.figma.Desktop"), "Figma");
        assert_eq!(pretty_bundle("Google"), "Google");
    }
}
