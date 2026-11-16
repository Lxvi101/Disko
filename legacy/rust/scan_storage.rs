//! Retain one completed scan; a replacement is temporary until it is validated.
use rusqlite::{Connection, OpenFlags};
use std::{fs, path::{Path, PathBuf}, time::UNIX_EPOCH};

/// Failed, cancelled, or aborted replacements must not leave a second database behind.
pub struct PendingScan(PathBuf);
impl PendingScan {
    pub fn new(path: PathBuf) -> Self { Self(path) }
}
impl Drop for PendingScan {
    fn drop(&mut self) {
        let active = self.0.parent().and_then(|dir| fs::read_to_string(dir.join(".active")).ok());
        let committed = active.as_deref().map(str::trim) == self.0.file_name().and_then(|s| s.to_str());
        if !committed {
            if let Err(e) = remove_database(&self.0) { eprintln!("{}", e); }
        }
    }
}

fn scan_state(path: &Path) -> Option<(bool, f64)> {
    if !fs::symlink_metadata(path).ok()?.file_type().is_file() { return None; }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.prepare("SELECT id FROM entries LIMIT 0").ok()?;
    let value = |key: &str| -> Option<serde_json::Value> {
        let raw: String = conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).ok()?;
        serde_json::from_str(&raw).ok()
    };
    if value("root")?.as_str()?.is_empty() { return None; }
    let complete = value("complete").and_then(|v| v.as_bool()).unwrap_or(false);
    let modified = fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    let finished = value("finished").and_then(|v| v.as_f64()).filter(|v| v.is_finite()).unwrap_or(modified);
    Some((complete, finished))
}

fn scan_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let regular = fs::symlink_metadata(&path).map(|m| m.file_type().is_file()).unwrap_or(false);
        // A crash can leave an unreadable database. Recognize only our own generated names
        // in that case; an unrelated SQLite file in this directory is never removed.
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let generated = name.strip_suffix(".sqlite").and_then(|s| s.rsplit_once('-'))
            .is_some_and(|(prefix, stamp)| !stamp.is_empty() && stamp.bytes().all(|c| c.is_ascii_digit())
                && (prefix.ends_with("-quick") || prefix.ends_with("-full")));
        let owned = matches!(name, "home.sqlite" | "home-prev.sqlite") || generated;
        if regular && path.extension().is_some_and(|ext| ext == "sqlite") && (owned || scan_state(&path).is_some()) {
            paths.push(path);
        }
    }
    Ok(paths)
}

pub fn remove_database(path: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match fs::remove_file(PathBuf::from(name)) {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("Could not remove scan {}: {}", path.display(), e)),
        }
    }
    Ok(())
}

/// Switch only to a completed scan, then prune all other scan databases and SQLite sidecars.
pub fn activate_and_prune(dir: &Path, keep: &Path) -> Result<(), String> {
    if keep.parent() != Some(dir) || !scan_state(keep).is_some_and(|s| s.0) {
        return Err("The replacement scan is not complete or cannot be read".into());
    }
    let name = keep.file_name().ok_or("Invalid scan path")?.to_string_lossy();
    let changed = fs::read_to_string(dir.join(".active")).unwrap_or_default().trim() != name;
    fs::write(dir.join(".active-next"), name.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(dir.join(".active-next"), dir.join(".active")).map_err(|e| e.to_string())?;
    if changed {
        // These reports belong to the previous database and will be regenerated on demand.
        for name in ["candidates.json", "unused-files.json", "remnants.json"] {
            let path = dir.join(name);
            if path.exists() { fs::remove_file(path).map_err(|e| e.to_string())?; }
        }
    }
    for path in scan_files(dir)? {
        if path != keep { remove_database(&path)?; }
    }
    Ok(())
}

/// Migrate older installations, including the formerly hidden home-prev.sqlite backup.
pub fn retain_latest(dir: &Path) -> Result<(), String> {
    let paths = scan_files(dir)?;
    let latest = paths.iter().filter_map(|p| {
        let (complete, finished) = scan_state(p)?;
        complete.then_some((p, finished))
    }).max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    if let Some((keep, _)) = latest {
        activate_and_prune(dir, keep)
    } else {
        for path in paths { remove_database(&path)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("disko-retention-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&dir).unwrap(); Self(dir)
        }
        fn scan(&self, name: &str, complete: bool, finished: u64) -> PathBuf {
            let path = self.0.join(name);
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE entries(id INTEGER); CREATE TABLE meta(key TEXT, value TEXT);").unwrap();
            for (k,v) in [("root", "\"/Users/test\"".to_string()), ("complete", complete.to_string()), ("finished", finished.to_string())] {
                c.execute("INSERT INTO meta VALUES (?1, ?2)", [k, &v]).unwrap();
            }
            path
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn startup_keeps_newest_complete_and_removes_backup_and_partial() {
        let f = Fixture::new();
        let old = f.scan("home-prev.sqlite", true, 10);
        let latest = f.scan("home-quick-20.sqlite", true, 20);
        let partial = f.scan("home-full-30.sqlite", false, 30);
        fs::write(f.0.join("home-prev.sqlite-journal"), "sidecar").unwrap();
        fs::write(f.0.join("notes.txt"), "keep").unwrap();
        fs::write(f.0.join("unrelated.sqlite"), "not a scan").unwrap();
        fs::write(f.0.join(".active"), "home-prev.sqlite").unwrap();
        retain_latest(&f.0).unwrap();
        assert!(latest.exists()); assert!(!old.exists()); assert!(!partial.exists());
        assert!(!f.0.join("home-prev.sqlite-journal").exists());
        assert!(f.0.join("notes.txt").exists()); assert!(f.0.join("unrelated.sqlite").exists());
        assert_eq!(fs::read_to_string(f.0.join(".active")).unwrap(), "home-quick-20.sqlite");
    }
    #[test]
    fn failed_replacement_preserves_last_complete_scan() {
        let f = Fixture::new();
        let old = f.scan("old.sqlite", true, 10);
        let partial = f.scan("partial.sqlite", false, 20);
        fs::write(f.0.join(".active"), "old.sqlite").unwrap();
        assert!(activate_and_prune(&f.0, &partial).is_err());
        remove_database(&partial).unwrap();
        assert!(old.exists());
        assert_eq!(fs::read_to_string(f.0.join(".active")).unwrap(), "old.sqlite");
    }
    #[test]
    fn replacement_removes_old_scan_and_stale_reports() {
        let f = Fixture::new();
        let old = f.scan("old.sqlite", true, 10);
        let new = f.scan("new.sqlite", true, 20);
        fs::write(f.0.join("candidates.json"), "{}").unwrap();
        activate_and_prune(&f.0, &new).unwrap();
        assert!(new.exists()); assert!(!old.exists()); assert!(!f.0.join("candidates.json").exists());
        retain_latest(&f.0).unwrap();
        assert_eq!(scan_files(&f.0).unwrap(), vec![new]);
    }
    #[test]
    fn removes_all_sidecars_even_if_main_file_is_already_gone() {
        let f = Fixture::new();
        for suffix in ["-wal", "-shm", "-journal"] { fs::write(f.0.join(format!("partial.sqlite{}", suffix)), "x").unwrap(); }
        remove_database(&f.0.join("partial.sqlite")).unwrap();
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }
    #[test]
    fn pending_guard_discards_failed_scan_but_keeps_published_scan() {
        let f = Fixture::new();
        let partial = f.scan("partial.sqlite", false, 10);
        { let _pending = PendingScan::new(partial.clone()); }
        assert!(!partial.exists());
        let complete = f.scan("complete.sqlite", true, 20);
        {
            let _pending = PendingScan::new(complete.clone());
            activate_and_prune(&f.0, &complete).unwrap();
        }
        assert!(complete.exists());
    }
    #[test]
    fn no_completed_scan_leaves_no_partial_database() {
        let f = Fixture::new();
        let partial = f.scan("partial.sqlite", false, 10);
        retain_latest(&f.0).unwrap();
        assert!(!partial.exists());
    }
    #[test]
    fn crash_cleanup_removes_corrupt_generated_scan_but_not_unrelated_files() {
        let f = Fixture::new();
        fs::write(f.0.join("home-full-123.sqlite"), "interrupted write").unwrap();
        fs::write(f.0.join("other.sqlite"), "keep").unwrap();
        retain_latest(&f.0).unwrap();
        assert!(!f.0.join("home-full-123.sqlite").exists());
        assert!(f.0.join("other.sqlite").exists());
    }
}
