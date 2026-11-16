//! Deep rules from the MacBook clean: evidence that needs content checks, not just a location.

use super::MIN;
use crate::apps::read_app;
use crate::finding::{make, FindingExt};
use crate::util::{child_dirs, children, du, du_all, exists, is_dir, is_file, mtime, name_of, now, parse_version, s, version_str, Size, DAY, GB};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes hashed per scan, across all content checks.
const HASH_BUDGET: u64 = 8 * GB;

fn hash_reader(r: &mut dyn Read, budget: &AtomicU64) -> Result<String, String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if budget.fetch_add(n as u64, Ordering::Relaxed) + n as u64 > HASH_BUDGET {
            return Err("hash budget reached".into());
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

pub fn hash_file(p: &Path, budget: &AtomicU64) -> Result<String, String> {
    let before = fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("not a regular file".into());
    }
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let h = hash_reader(&mut f, budget)?;
    let after = fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if (before.ino(), before.len(), before.mtime(), before.mtime_nsec()) != (after.ino(), after.len(), after.mtime(), after.mtime_nsec()) {
        return Err("changed while reading".into());
    }
    Ok(h)
}

/// Every regular member of `archive` exists in `extracted` with identical contents (SHA-256).
/// Accepts members laid out flat or under one enclosing folder named like `extracted`. Rejects
/// symlinks, traversal, mixed layouts and empty archives. Returns the number of files compared.
pub fn verify_zip(archive: &Path, extracted: &Path, budget: &AtomicU64) -> Result<usize, String> {
    let extracted = fs::canonicalize(extracted).map_err(|e| e.to_string())?;
    let before = fs::symlink_metadata(archive).map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("not a regular file".into());
    }
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > 20_000 {
        return Err("too many members".into());
    }
    let enclosing = extracted.file_name().ok_or("no name")?.to_os_string();
    let mut names = vec![];
    for i in 0..zip.len() {
        let m = zip.by_index(i).map_err(|e| e.to_string())?;
        if m.is_dir() {
            continue;
        }
        let n = m.enclosed_name().ok_or("unsafe member path")?;
        if n.starts_with("__MACOSX") {
            continue;
        }
        names.push(n);
    }
    let prefixed = names.iter().filter(|p| p.components().next().map(|c| c.as_os_str() == enclosing).unwrap_or(false)).count();
    if prefixed > 0 && prefixed != names.len() {
        return Err("mixed layout".into());
    }
    let strip = prefixed > 0;
    let mut matched = 0;
    for i in 0..zip.len() {
        let mut m = zip.by_index(i).map_err(|e| e.to_string())?;
        if m.is_dir() {
            continue;
        }
        if m.unix_mode().is_some_and(|x| x & 0o170000 == 0o120000) {
            return Err("symlink member".into());
        }
        let rel = m.enclosed_name().ok_or("unsafe member path")?;
        if rel.starts_with("__MACOSX") {
            continue;
        }
        let rel = if strip { rel.strip_prefix(&enclosing).map_err(|_| "layout")?.to_path_buf() } else { rel };
        if rel.as_os_str().is_empty() {
            return Err("empty member path".into());
        }
        let target = extracted.join(&rel);
        let real = fs::canonicalize(&target).map_err(|_| "missing in extracted copy")?;
        if real != target {
            return Err("extracted path goes through a symlink".into());
        }
        let tm = fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
        if !tm.is_file() || tm.len() != m.size() {
            return Err("extracted contents differ".into());
        }
        if hash_reader(&mut m, budget)? != hash_file(&target, budget)? {
            return Err("extracted contents differ".into());
        }
        matched += 1;
    }
    let after = fs::symlink_metadata(archive).map_err(|e| e.to_string())?;
    if (before.ino(), before.len(), before.mtime()) != (after.ino(), after.len(), after.mtime()) {
        return Err("archive changed".into());
    }
    if matched == 0 {
        return Err("nothing to compare".into());
    }
    Ok(matched)
}

/// ZIPs kept next to the folder they were extracted into, verified file by file.
pub fn archives(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let budget = AtomicU64::new(0);
    let mut dirs: Vec<PathBuf> = vec![home.join("Downloads"), home.join("Desktop"), home.join("Documents")];
    let downloads = if c.env.readable(&home.join("Downloads")) { child_dirs(&home.join("Downloads")) } else { vec![] };
    for d in downloads.into_iter().chain(child_dirs(&home.join(".cache"))) {
        let n = name_of(&d).to_lowercase();
        if !["uv", "pip", "npm", "mcp", "modelcontextprotocol", "browser", "huggingface"].iter().any(|x| n.contains(x)) {
            dirs.push(d);
        }
    }
    dirs.retain(|d| c.env.readable(d));
    let mut out = vec![];
    for dir in dirs {
        for p in children(&dir) {
            let n = name_of(&p);
            if !n.to_ascii_lowercase().ends_with(".zip") || !is_file(&p) {
                continue;
            }
            let stem = &n[..n.len() - 4];
            let extracted = dir.join(stem);
            if !is_dir(&extracted) {
                continue;
            }
            match verify_zip(&p, &extracted, &budget) {
                Ok(files) => {
                    let size = du(&p);
                    if size.bytes < MIN / 4 {
                        continue;
                    }
                    out.push(
                        make("installers.extracted-zip", Category::Installers, n.clone(), "box", vec![s(&p)], size, Safety::Review, false, format!("All {} files in it are already unpacked, byte-for-byte identical, in “{}” next to it.", files, stem))
                            .detail(disko_core::paths::short(&s(&p))),
                    );
                }
                Err(_) => continue,
            }
        }
    }
    out
}

/// Adobe: support payloads for app versions no longer installed, nested installer apps,
/// extracted installer staging.
pub fn adobe(c: &Clutter) -> Vec<Finding> {
    let apps_dir = c.env.sys("/Applications");
    let mut installed: Vec<(String, String, String)> = vec![];
    let mut unreadable = false;
    for root in children(&apps_dir).into_iter().filter(|p| name_of(p).starts_with("Adobe ")) {
        let entries = if name_of(&root).ends_with(".app") { vec![root] } else { children(&root) };
        for p in entries.into_iter().filter(|p| name_of(p).ends_with(".app")) {
            match read_app(&p) {
                Some(a) => installed.push((s(&p), a.id, a.version.unwrap_or_default())),
                None => unreadable = true,
            }
        }
    }
    let mut out = vec![];
    let base = c.env.sys("/Library/Application Support/Adobe");
    if !unreadable && !installed.is_empty() {
        for (family, fragment, beta) in [("Premiere Pro", "premierepro", false), ("Adobe Media Encoder", "ame.application", false), ("Media Encoder", "ame.application", false), ("Adobe Media Encoder (Beta)", "ame.application", true), ("After Effects", "aftereffects", false)] {
            let versions: HashSet<String> = installed
                .iter()
                .filter(|(p, id, _)| id.to_lowercase().contains(fragment) && p.to_lowercase().contains("beta") == beta)
                .filter_map(|(_, _, v)| v.split('.').next().map(str::to_string))
                .collect();
            for p in child_dirs(&base.join(family)) {
                let n = name_of(&p);
                let Some(major) = n.split('.').next() else { continue };
                if major.parse::<u32>().is_err() || versions.contains(major) {
                    continue;
                }
                // Only known downloaded payloads, never licensing or arbitrary state.
                if !["SpeechESL", "MediaIntelligence", "EnhanceSpeech", "Neural Filters"].iter().any(|x| is_dir(&p.join(x))) {
                    continue;
                }
                let size = du(&p);
                if size.bytes < MIN {
                    continue;
                }
                let mut f = make("leftovers.adobe-version", Category::Leftovers, format!("{} {}", family, n), "app", vec![s(&p)], size, Safety::Review, false, format!("Speech and media models for {} {}; installed versions are {}.", family, major, if versions.is_empty() { "none".to_string() } else { versions.iter().cloned().collect::<Vec<_>>().join(", ") }))
                    .detail("Adobe support payload");
                f.admin = crate::util::needs_admin(&p);
                out.push(f);
            }
        }
    }
    // Installer apps kept inside Keyfiles (repair may need them; the Keyfiles folder stays).
    let keyfiles = base.join("Keyfiles/AfterEffects");
    for v in child_dirs(&keyfiles) {
        for arch in ["arm64", "x64"] {
            for p in child_dirs(&v.join("CustomHooks/mac").join(arch)) {
                let n = name_of(&p);
                if !n.ends_with("Installer.app") || !is_file(&p.join("Contents/Resources/installbuilder")) {
                    continue;
                }
                let size = du(&p);
                if size.bytes < MIN {
                    continue;
                }
                let mut f = make("installers.adobe-nested", Category::Installers, format!("After Effects {}", n.trim_end_matches(".app")), "app", vec![s(&p)], size, Safety::Review, false, "An installer bundle Adobe kept after installing; a repair may need to download it again.").detail("Adobe installer");
                f.admin = crate::util::needs_admin(&p);
                out.push(f);
            }
        }
    }
    // Extracted installation staging.
    let temp = c.env.sys("/.adobeTemp");
    for p in child_dirs(&temp) {
        let staging = walk_names(&p, 4).iter().any(|n| n.starts_with("StagingArea"));
        if staging {
            let size = du(&p);
            let mut f = make("installers.adobe-staging", Category::Installers, "Adobe installer staging", "app", vec![s(&p)], size, Safety::Review, false, "Files an Adobe installer unpacked and left behind. Don't remove while Creative Cloud is updating.");
            f.admin = true;
            f.busy = c.procs().named(&["Adobe Installer", "HDHelper", "Install"]).map(|p| format!("{} is running", crate::procs::who(p)));
            out.push(f);
        }
    }
    out
}

fn walk_names(dir: &Path, depth: u8) -> Vec<String> {
    let mut out = vec![];
    for p in children(dir) {
        out.push(name_of(&p));
        if depth > 0 && is_dir(&p) {
            out.extend(walk_names(&p, depth - 1));
        }
    }
    out
}

/// Duplicate compiled speech models: same size and SHA-256, at least a week old; the newest
/// copy of each stays.
pub fn speech(c: &Clutter) -> Vec<Finding> {
    let root = c.env.sys("/private/var/tmp/SpeechModelCache");
    let budget = AtomicU64::new(0);
    let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
    for p in children(&root) {
        if name_of(&p).ends_with(".bnnsir") {
            if let Ok(m) = fs::symlink_metadata(&p) {
                if m.is_file() && m.len() > 0 {
                    by_size.entry(m.len()).or_default().push(p);
                }
            }
        }
    }
    let mut dups = vec![];
    for files in by_size.values().filter(|v| v.len() > 1) {
        let mut by_hash: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for p in files {
            if let Ok(h) = hash_file(p, &budget) {
                by_hash.entry(h).or_default().push(p.clone());
            }
        }
        for (_, mut same) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
            same.sort_by_key(|p| std::cmp::Reverse(mtime(p).unwrap_or(0)));
            dups.extend(same.into_iter().skip(1).filter(|p| mtime(p).is_some_and(|t| now() - t >= 7 * DAY)));
        }
    }
    if dups.is_empty() {
        return vec![];
    }
    let size = du_all(&dups);
    if size.bytes < MIN {
        return vec![];
    }
    let mut f = make("cache.speech-duplicates", Category::Cache, "Speech model copies", "box", dups.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, format!("{} compiled speech models identical (SHA-256) to a newer copy that stays; macOS may compile them again.", dups.len())).keyed("speech-duplicates");
    f.admin = dups.iter().any(|p| crate::util::needs_admin(p));
    vec![f]
}

/// TeX Live keeps a full tree per yearly release.
pub fn texlive(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    for (root, user) in [(c.env.sys("/usr/local/texlive"), false), (c.env.home.join("Library/texlive"), true)] {
        let mut years: Vec<(Vec<u64>, PathBuf)> = child_dirs(&root).into_iter().filter_map(|p| Some((parse_version(&name_of(&p)).filter(|v| v[0] > 2000)?, p))).collect();
        if years.len() < 2 {
            continue;
        }
        years.sort();
        let newest = years.pop().unwrap();
        let paths: Vec<PathBuf> = years.iter().map(|x| x.1.clone()).collect();
        let size = du_all(&paths);
        if size.bytes < MIN {
            continue;
        }
        let names: Vec<String> = years.iter().map(|x| version_str(&x.0)).collect();
        let mut f = make("versions.texlive", Category::OldVersions, if user { "TeX Live user files" } else { "TeX Live" }, "box", paths.iter().map(|p| s(p)).collect(), size, if user { Safety::Safe } else { Safety::Review }, false, format!("TeX Live {} is installed; {} are earlier yearly releases left beside it.", version_str(&newest.0), names.join(", ")))
            .keyed(&s(&root))
            .detail(names.join(", "));
        f.admin = !user;
        out.push(f);
    }
    let _ = (exists, Size::default());
    out
}
