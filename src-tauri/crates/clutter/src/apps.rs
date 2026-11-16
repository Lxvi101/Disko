//! Which apps are installed (by bundle id). The leftovers, installers and cache rules use it to
//! name things and to tell what is no longer owned by anything.

use crate::util::{child_dirs, name_of, plist_dict, plist_str};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub path: PathBuf,
    /// Lowercased bundle id.
    pub id: String,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Apps {
    pub list: Vec<App>,
    by_id: HashMap<String, usize>,
    vendors: HashSet<String>,
}

/// Hosting namespaces say nothing about who made an app.
const GENERIC_VENDORS: &[&str] = &["com.github", "io.github", "com.gitlab", "com.electron", "com.example", "com.yourcompany", "org.example", "com.todesktop", "dev.tauri"];

pub fn vendor_key(id: &str) -> String {
    let parts: Vec<&str> = id.split('.').collect();
    let two = parts.iter().take(2).cloned().collect::<Vec<_>>().join(".");
    if GENERIC_VENDORS.contains(&two.as_str()) {
        parts.iter().take(3).cloned().collect::<Vec<_>>().join(".")
    } else {
        two
    }
}

pub fn read_app(path: &Path) -> Option<App> {
    let d = plist_dict(&path.join("Contents/Info.plist"))?;
    let id = plist_str(&d, "CFBundleIdentifier")?.trim().to_ascii_lowercase();
    if id.is_empty() {
        return None;
    }
    let name = plist_str(&d, "CFBundleDisplayName")
        .or_else(|| plist_str(&d, "CFBundleName"))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| name_of(path).trim_end_matches(".app").to_string());
    Some(App { path: path.to_path_buf(), id, name, version: plist_str(&d, "CFBundleShortVersionString") })
}

impl Apps {
    /// Apps in the usual folders under `root` ("/" in production) and in `home/Applications`.
    pub fn scan(root: &Path, home: &Path) -> Apps {
        let mut list = vec![];
        let mut seen = HashSet::new();
        let dirs = [
            (root.join("Applications"), 2u8),
            (root.join("System/Applications"), 1),
            (root.join("System/Library/CoreServices"), 0),
            (home.join("Applications"), 2),
            (root.join("Library/Application Support"), 1),
        ];
        fn walk(dir: &Path, depth: u8, out: &mut Vec<App>, seen: &mut HashSet<PathBuf>) {
            for p in child_dirs(dir) {
                let n = name_of(&p);
                if n.ends_with(".app") {
                    if seen.insert(p.clone()) {
                        if let Some(a) = read_app(&p) {
                            out.push(a);
                        }
                    }
                } else if depth > 0 && !n.starts_with('.') && !n.ends_with(".bundle") && !n.ends_with(".framework") {
                    walk(&p, depth - 1, out, seen);
                }
            }
        }
        for (d, depth) in dirs {
            walk(&d, depth, &mut list, &mut seen);
        }
        Apps::from_list(list)
    }

    pub fn from_list(list: Vec<App>) -> Apps {
        let by_id = list.iter().enumerate().map(|(i, a)| (a.id.clone(), i)).collect();
        let vendors = list.iter().map(|a| vendor_key(&a.id)).collect();
        Apps { list, by_id, vendors }
    }

    pub fn get(&self, id: &str) -> Option<&App> {
        self.by_id.get(&id.to_ascii_lowercase()).map(|i| &self.list[*i])
    }

    /// An installed app owns this id: same id, or one is a dotted prefix of the other
    /// ("com.foo.app.helper" belongs to "com.foo.app").
    pub fn owner(&self, id: &str) -> Option<&App> {
        let id = id.to_ascii_lowercase();
        if let Some(a) = self.get(&id) {
            return Some(a);
        }
        self.list.iter().find(|a| id.starts_with(&format!("{}.", a.id)) || a.id.starts_with(&format!("{}.", id)))
    }

    pub fn vendor_installed(&self, id: &str) -> bool {
        self.vendors.contains(&vendor_key(&id.to_ascii_lowercase()))
    }

    /// An installed app whose name matches (case-insensitive, ignoring spaces): used for
    /// installers ("Docker.dmg" → Docker.app) and cache folders named after an app.
    pub fn by_name(&self, name: &str) -> Option<&App> {
        let key = squash(name);
        if key.len() < 3 {
            return None;
        }
        self.list.iter().find(|a| squash(&a.name) == key || squash(&name_of(&a.path).trim_end_matches(".app").to_string()) == key)
    }
}

pub fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// True when `s` looks like a reverse-DNS bundle identifier (three or more components).
pub fn is_bundle_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        && parts[0].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}
