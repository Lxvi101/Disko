//! `memory.json` on disk: tolerant load, atomic locked writes, mtime-based reload so the app sees
//! what an agent changed through the MCP process (and the other way round).

use crate::community::{self, Pack};
use crate::share::{self, SubmitReport, Submission};
use crate::{now, rules, short_hash, slug, KeepEntry, KeepSet, LearnedRule, Memory, MemoryOp, Routine, SCHEMA_VERSION};
use disko_core::Source;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const FILE: &str = "memory.json";
const DAY: i64 = 86_400;

/// Disko's support folder: `DISKO_SUPPORT_DIR` if set (tests, portable setups), else
/// `~/Library/Application Support/app.disko`.
pub fn default_dir() -> PathBuf {
    match std::env::var_os("DISKO_SUPPORT_DIR") {
        Some(d) if !d.is_empty() => {
            let d = PathBuf::from(d);
            let _ = std::fs::create_dir_all(&d);
            d
        }
        _ => disko_core::paths::support_dir(),
    }
}

/// Write a file atomically (temp file in the same folder, fsync, rename).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        std::process::id()
    ));
    let res = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(|e| format!("Couldn't write {}: {}", path.display(), e))
}

/// Exclusive lock on `memory.lock` for read-modify-write across processes.
struct Lock(#[allow(dead_code)] std::fs::File);

impl Lock {
    fn take(dir: &Path) -> Result<Lock, String> {
        use std::os::fd::AsRawFd;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("memory.lock"))
            .map_err(|e| e.to_string())?;
        // SAFETY: valid fd owned by `f`; the lock is released when `f` is closed.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err("Couldn't lock memory.json".into());
        }
        Ok(Lock(f))
    }
}

type Stamp = Option<(SystemTime, u64)>;

fn stamp_of(p: &Path) -> Stamp {
    std::fs::metadata(p).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
}

/// Read `memory.json`. Unknown fields are ignored, broken entries dropped one by one; an
/// unreadable file is moved aside (`memory.json.bad-<time>`) and Disko starts fresh. Returns the
/// memory and whether the file was written by a newer Disko (then we refuse to overwrite it).
fn read(path: &Path) -> (Memory, bool) {
    let fresh = || Memory { routines: crate::routines::suggested(), ..Default::default() };
    let Ok(bytes) = std::fs::read(path) else { return (fresh(), false) };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        let _ = std::fs::rename(path, path.with_extension(format!("json.bad-{}", now())));
        return (fresh(), false);
    };
    let newer = v.get("schema_version").and_then(Value::as_u64).unwrap_or(1) > SCHEMA_VERSION as u64;
    fn list<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Vec<T> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect())
            .unwrap_or_default()
    }
    let m = Memory {
        keep: list(&v, "keep"),
        rules: list(&v, "rules"),
        routines: list(&v, "routines"),
        community: v.get("community").and_then(|c| serde_json::from_value(c.clone()).ok()).unwrap_or_default(),
    };
    (m, newer)
}

pub struct Store {
    dir: PathBuf,
    home: PathBuf,
    user: String,
    mem: Memory,
    stamp: Stamp,
    newer: bool,
    keep: KeepSet,
    pack: Option<Pack>,
}

impl Store {
    /// The real store (see [`default_dir`]).
    pub fn open_default() -> Store {
        Store::open(default_dir(), disko_core::paths::home())
    }

    /// A store in `dir`, expanding `~` with `home` (tests pass temp dirs for both).
    pub fn open(dir: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Store {
        let home = home.into();
        let user = home.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut s = Store {
            dir: dir.into(),
            home,
            user,
            mem: Memory::default(),
            stamp: None,
            newer: false,
            keep: KeepSet::default(),
            pack: None,
        };
        s.reload();
        s
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    fn file(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    fn reload(&mut self) {
        let f = self.file();
        self.stamp = stamp_of(&f);
        let (m, newer) = read(&f);
        self.mem = m;
        self.newer = newer;
        self.keep = KeepSet::new(&self.mem.keep, &self.home);
    }

    /// Reload if another process changed the file. Cheap (one stat); returns true if it changed.
    pub fn refresh(&mut self) -> bool {
        if stamp_of(&self.file()) != self.stamp {
            self.reload();
            true
        } else {
            false
        }
    }

    /// Current memory, as the UI and agents see it (community counts filled in).
    pub fn memory(&mut self) -> Memory {
        self.refresh();
        let mut m = self.mem.clone();
        m.community.rules = if m.community.enabled { self.pack().rules.len() } else { 0 };
        m.community.queued = share::outbox(&self.dir).len();
        m
    }

    /// The compiled keep list (refreshed). Check findings with `keep().finding_kept_by(f)`.
    pub fn keep(&mut self) -> &KeepSet {
        self.refresh();
        &self.keep
    }

    pub fn is_kept(&mut self, path: &str) -> bool {
        self.keep().is_kept(path)
    }

    fn pack(&mut self) -> &Pack {
        if self.pack.is_none() {
            self.pack = Some(community::load(&self.dir));
        }
        self.pack.as_ref().unwrap()
    }

    /// Rules to scan with: the user's/agents' rules, plus the community pack when enabled (minus
    /// community rules with the same pattern as a local one).
    pub fn active_rules(&mut self) -> Vec<LearnedRule> {
        self.refresh();
        let mut v = self.mem.rules.clone();
        if self.mem.community.enabled {
            let own: Vec<String> = v.iter().map(|r| r.pattern.clone()).collect();
            let extra: Vec<LearnedRule> = self.pack().rules.iter().filter(|r| !own.contains(&r.pattern)).cloned().collect();
            v.extend(extra);
        }
        v
    }

    pub fn due_routines(&mut self, now: i64) -> Vec<Routine> {
        self.refresh();
        self.mem.routines.iter().filter(|r| r.due(now)).cloned().collect()
    }

    /// Locked read-modify-write of the file.
    fn update<T>(&mut self, f: impl FnOnce(&mut Memory, &Ctx) -> Result<T, String>) -> Result<T, String> {
        let _lock = Lock::take(&self.dir)?;
        self.reload();
        if self.newer {
            return Err("memory.json was written by a newer Disko; update Disko to change it.".into());
        }
        let mut m = self.mem.clone();
        let ctx = Ctx { dir: self.dir.clone(), home: self.home.clone(), user: self.user.clone() };
        let out = f(&mut m, &ctx)?;
        let mut v = serde_json::to_value(&m).map_err(|e| e.to_string())?;
        if let Value::Object(o) = &mut v {
            o.insert("schema_version".into(), SCHEMA_VERSION.into());
            if let Some(Value::Object(c)) = o.get_mut("community") {
                c.remove("rules");
                c.remove("queued");
            }
        }
        let bytes = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
        write_atomic(&self.file(), &bytes)?;
        self.reload();
        Ok(out)
    }

    pub fn apply(&mut self, op: MemoryOp) -> Result<Memory, String> {
        self.apply_with(op, None)
    }

    /// Apply a change. `bytes` (what the rule matches now, if known) only feeds the size bucket of
    /// a shared rule's anonymous submission.
    pub fn apply_with(&mut self, op: MemoryOp, bytes: Option<u64>) -> Result<Memory, String> {
        self.update(|m, ctx| apply_op(m, ctx, op, bytes))?;
        Ok(self.memory())
    }

    /// Remember a routine run.
    pub fn record_run(&mut self, id: &str, at: i64, freed: u64) -> Result<(), String> {
        self.update(|m, _| {
            let r = m.routines.iter_mut().find(|r| r.id == id).ok_or_else(|| format!("No routine `{}`.", id))?;
            r.last_run = Some(at);
            r.last_freed = Some(freed);
            Ok(())
        })
    }

    /// Fetch the latest community pack if enabled and not checked in the last day (or `force`).
    /// Returns whether a new pack was loaded.
    pub fn refresh_community(&mut self, now: i64, force: bool) -> Result<bool, String> {
        self.refresh_community_with(now, force, community::http_get)
    }

    pub fn refresh_community_with(
        &mut self,
        now: i64,
        force: bool,
        get: impl Fn(&str) -> Result<Vec<u8>, String>,
    ) -> Result<bool, String> {
        self.refresh();
        if !self.mem.community.enabled {
            return Ok(false);
        }
        if !force && self.mem.community.checked.is_some_and(|t| now - t < DAY && now >= t) {
            return Ok(false);
        }
        let url = std::env::var("DISKO_COMMUNITY_PACK_URL").unwrap_or_else(|_| community::PACK_URL.to_string());
        let res = community::fetch_with(&self.dir, &url, get);
        let ok = res.is_ok();
        self.update(|m, _| {
            m.community.checked = Some(now);
            if ok {
                m.community.updated = Some(now);
            }
            Ok(())
        })?;
        self.pack = None;
        res.map(|_| true)
    }

    pub fn outbox(&self) -> Vec<Submission> {
        share::outbox(&self.dir).into_iter().map(|(_, s)| s).collect()
    }

    /// Send queued anonymous submissions if an endpoint is configured; otherwise they stay queued.
    pub fn submit_outbox(&mut self) -> SubmitReport {
        self.refresh();
        let ep = share::endpoint(self.mem.community.submit_url.as_deref());
        share::submit_with(&self.dir, ep.as_deref(), share::http_post_json)
    }
}

struct Ctx {
    dir: PathBuf,
    home: PathBuf,
    user: String,
}

impl Ctx {
    fn queue(&self, r: &LearnedRule, bytes: Option<u64>) -> Result<(), String> {
        share::enqueue(&self.dir, &share::anonymize(r, &self.home, &self.user, bytes)).map(|_| ())
    }
    fn unqueue(&self, r: &LearnedRule) {
        share::dequeue(&self.dir, &share::generalize(&r.pattern, &self.home, &self.user));
    }
}

fn apply_op(m: &mut Memory, ctx: &Ctx, op: MemoryOp, bytes: Option<u64>) -> Result<(), String> {
    let t = now();
    match op {
        MemoryOp::Keep { pattern, note, source } => {
            let pattern = pattern.trim().trim_end_matches('/').to_string();
            let g = crate::Glob::new(&pattern, &ctx.home)?;
            if g.segs.len() <= crate::glob::segments(&ctx.home.to_string_lossy()).len() && pattern.starts_with('~')
                || g.segs.len() < 2
            {
                return Err("Too broad: that would keep everything.".into());
            }
            if let Some(e) = m.keep.iter_mut().find(|e| e.pattern == pattern) {
                if note.is_some() {
                    e.note = note;
                }
                return Ok(());
            }
            m.keep.push(KeepEntry {
                id: format!("k-{}", short_hash(&pattern)),
                pattern,
                note: note.map(|n| n.chars().take(200).collect()),
                source: source.filter(|s| *s == Source::Agent).unwrap_or(Source::User),
                added: t,
            });
        }
        MemoryOp::Unkeep { id } => {
            let before = m.keep.len();
            let id = id.trim().trim_end_matches('/');
            m.keep.retain(|e| e.id != id && e.pattern != id);
            if m.keep.len() == before {
                return Err(format!("Nothing on the keep list matches `{}`.", id));
            }
        }
        MemoryOp::AddRule { mut rule } => {
            rule.pattern = rule.pattern.trim().trim_end_matches('/').to_string();
            rule.title = rule.title.trim().to_string();
            if matches!(rule.source, Source::Builtin | Source::Community) {
                rule.source = Source::User;
            }
            let existing = m.rules.iter().position(|r| (!rule.id.is_empty() && r.id == rule.id) || r.pattern == rule.pattern);
            if rule.id.is_empty() {
                rule.id = match existing {
                    Some(i) => m.rules[i].id.clone(),
                    None => {
                        let base = if slug(&rule.title).is_empty() { "rule".to_string() } else { slug(&rule.title) };
                        if m.rules.iter().any(|r| r.id == base) {
                            format!("{}-{}", base, &short_hash(&rule.pattern)[..4])
                        } else {
                            base
                        }
                    }
                };
            }
            rules::validate_rule(&rule, &disko_core::paths::home())?;
            if rule.added == 0 {
                rule.added = existing.map(|i| m.rules[i].added).unwrap_or(t);
            }
            if let Some(i) = existing {
                let old = m.rules.remove(i);
                if old.shared {
                    ctx.unqueue(&old);
                }
            }
            if rule.shared {
                ctx.queue(&rule, bytes)?;
            }
            m.rules.push(rule);
        }
        MemoryOp::RemoveRule { id } => {
            let i = m.rules.iter().position(|r| r.id == id).ok_or_else(|| format!("No learned rule `{}`.", id))?;
            let old = m.rules.remove(i);
            if old.shared {
                ctx.unqueue(&old);
            }
        }
        MemoryOp::ShareRule { id, shared } => {
            let r = m.rules.iter_mut().find(|r| r.id == id).ok_or_else(|| format!("No learned rule `{}`.", id))?;
            r.shared = shared;
            if shared {
                ctx.queue(r, bytes)?;
            } else {
                ctx.unqueue(r);
            }
        }
        MemoryOp::SetRoutine { mut routine } => {
            routine.title = routine.title.trim().to_string();
            if routine.title.is_empty() || routine.title.chars().count() > 80 {
                return Err("A routine needs a short title.".into());
            }
            if routine.select.categories.is_empty() && routine.select.rules.is_empty() {
                return Err("A routine must select categories or rules.".into());
            }
            if !(1..=365).contains(&routine.every_days) || routine.min_age_days > 3650 {
                return Err("every_days must be 1–365, min_age_days ≤ 3650.".into());
            }
            if routine.id.is_empty() {
                routine.id = slug(&routine.title);
                if routine.id.is_empty() {
                    routine.id = format!("routine-{}", short_hash(&routine.title));
                }
            }
            if let Some(old) = m.routines.iter_mut().find(|r| r.id == routine.id) {
                if routine.last_run.is_none() {
                    routine.last_run = old.last_run;
                }
                if routine.last_freed.is_none() {
                    routine.last_freed = old.last_freed;
                }
                *old = routine;
            } else {
                m.routines.push(routine);
            }
        }
        MemoryOp::RemoveRoutine { id } => {
            let before = m.routines.len();
            m.routines.retain(|r| r.id != id);
            if m.routines.len() == before {
                return Err(format!("No routine `{}`.", id));
            }
        }
        MemoryOp::Community { enabled } => m.community.enabled = enabled,
        MemoryOp::CommunityUrl { url } => {
            let url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
            if let Some(u) = &url {
                if !u.starts_with("https://") {
                    return Err("The submission endpoint must be https://.".into());
                }
            }
            m.community.submit_url = url;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tempdir;
    use disko_core::{Category, Safety};

    fn rule(pattern: &str, title: &str) -> LearnedRule {
        LearnedRule {
            id: String::new(),
            pattern: pattern.into(),
            category: Category::Build,
            title: title.into(),
            why: "regrows".into(),
            regrows: true,
            safety: Safety::Safe,
            source: Source::Agent,
            added: 0,
            shared: false,
        }
    }

    #[test]
    fn fresh_store_has_suggested_routine_and_persists() {
        let d = tempdir("store");
        let mut s = Store::open(&d, "/Users/me");
        let m = s.memory();
        assert_eq!(m.routines.len(), 1);
        assert!(!m.routines[0].enabled);
        assert!(m.community.enabled);
        assert!(m.community.rules > 0);

        s.apply(MemoryOp::Keep { pattern: "~/Dev/app/node_modules/".into(), note: Some("offline".into()), source: Some(Source::Agent) })
            .unwrap();
        let m = s.apply(MemoryOp::AddRule { rule: rule("~/.turbo/cache", "Turbo cache") }).unwrap();
        assert_eq!(m.rules[0].id, "turbo-cache");
        assert!(m.rules[0].added > 0);
        assert!(s.is_kept("/Users/me/Dev/app/node_modules/x"));

        // Another process (a second Store) sees it, and the first sees changes from the second.
        let mut other = Store::open(&d, "/Users/me");
        assert_eq!(other.memory().keep[0].source, Source::Agent);
        let id = other.memory().keep[0].id.clone();
        std::thread::sleep(std::time::Duration::from_millis(5));
        other.apply(MemoryOp::Unkeep { id }).unwrap();
        assert!(!s.is_kept("/Users/me/Dev/app/node_modules/x"));

        let raw: Value = serde_json::from_slice(&std::fs::read(d.join(FILE)).unwrap()).unwrap();
        assert_eq!(raw["schema_version"], 1);
        assert!(raw["community"].get("rules").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_bad_ops() {
        let d = tempdir("store-bad");
        let mut s = Store::open(&d, "/Users/me");
        assert!(s.apply(MemoryOp::AddRule { rule: rule("~/*", "All") }).is_err());
        assert!(s.apply(MemoryOp::Keep { pattern: "~".into(), note: None, source: None }).is_err());
        assert!(s.apply(MemoryOp::Keep { pattern: "relative".into(), note: None, source: None }).is_err());
        assert!(s.apply(MemoryOp::Unkeep { id: "nope".into() }).is_err());
        assert!(s.apply(MemoryOp::RemoveRoutine { id: "nope".into() }).is_err());
        let bad = Routine { id: String::new(), title: "x".into(), select: Default::default(), every_days: 2, min_age_days: 0, enabled: true, review: false, last_run: None, last_freed: None };
        assert!(s.apply(MemoryOp::SetRoutine { routine: bad }).is_err());
        assert!(s.apply(MemoryOp::CommunityUrl { url: Some("http://evil".into()) }).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tolerant_load_and_newer_schema() {
        let d = tempdir("store-tolerant");
        std::fs::write(
            d.join(FILE),
            r#"{"schema_version":1,"keep":[{"id":"k1","pattern":"~/a/b","added":1},{"broken":true}],
                "rules":[{"id":"r","pattern":"~/.x/y","category":"no-such-category","title":"x","safety":"safe"}],
                "routines":[],"community":{"enabled":false},"future_field":42}"#,
        )
        .unwrap();
        let mut s = Store::open(&d, "/Users/me");
        let m = s.memory();
        assert_eq!(m.keep.len(), 1);
        assert_eq!(m.rules.len(), 0);
        assert!(!m.community.enabled);
        assert!(m.routines.is_empty(), "suggestions only seed a brand-new memory");

        std::fs::write(d.join(FILE), "{ not json").unwrap();
        let mut s = Store::open(&d, "/Users/me");
        assert_eq!(s.memory().routines.len(), 1);
        assert!(std::fs::read_dir(&d).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("bad-")));

        std::fs::write(d.join(FILE), r#"{"schema_version":99,"keep":[]}"#).unwrap();
        let mut s = Store::open(&d, "/Users/me");
        assert!(s.apply(MemoryOp::Community { enabled: false }).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn routines_and_runs() {
        let d = tempdir("store-routine");
        let mut s = Store::open(&d, "/Users/me");
        let mut r = s.memory().routines[0].clone();
        r.enabled = true;
        s.apply(MemoryOp::SetRoutine { routine: r }).unwrap();
        assert_eq!(s.due_routines(1000).len(), 1);
        s.record_run("build-artifacts", 1000, 123).unwrap();
        assert!(s.due_routines(1000 + DAY).is_empty());
        assert_eq!(s.due_routines(1000 + 2 * DAY).len(), 1);
        // Re-setting keeps the run history.
        let mut r = s.memory().routines[0].clone();
        r.last_run = None;
        r.every_days = 3;
        s.apply(MemoryOp::SetRoutine { routine: r }).unwrap();
        assert_eq!(s.memory().routines[0].last_freed, Some(123));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sharing_queues_anonymized_and_unshare_dequeues() {
        let d = tempdir("store-share");
        let mut s = Store::open(&d, "/Users/jdoe");
        let mut r = rule("/Users/jdoe/Clients/acme/.turbo", "Turbo output");
        r.shared = true;
        let m = s.apply_with(MemoryOp::AddRule { rule: r }, Some(2_000_000_000)).unwrap();
        assert_eq!(m.community.queued, 1);
        let ob = s.outbox();
        assert_eq!(ob[0].pattern, "~/**/.turbo");
        assert_eq!(ob[0].size.as_deref(), Some("1–10 GB"));
        let r = s.submit_outbox_test();
        assert_eq!(r.queued, 1);
        s.apply(MemoryOp::ShareRule { id: "turbo-output".into(), shared: false }).unwrap();
        assert!(s.outbox().is_empty());
        s.apply(MemoryOp::ShareRule { id: "turbo-output".into(), shared: true }).unwrap();
        s.apply(MemoryOp::RemoveRule { id: "turbo-output".into() }).unwrap();
        assert!(s.outbox().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn community_refresh_daily_and_toggle() {
        let d = tempdir("store-community");
        let mut s = Store::open(&d, "/Users/me");
        let pack = r#"{"schema_version":1,"updated":"2999-01-01","rules":[
            {"id":"only","pattern":"~/.gradle/caches","category":"packages","title":"Gradle","safety":"safe"}]}"#;
        let calls = std::cell::Cell::new(0);
        let get = |_: &str| {
            calls.set(calls.get() + 1);
            Ok(pack.as_bytes().to_vec())
        };
        assert!(s.refresh_community_with(10 * DAY, false, get).unwrap());
        assert_eq!(s.memory().community.rules, 1);
        assert!(!s.refresh_community_with(10 * DAY + 100, false, get).unwrap());
        assert_eq!(calls.get(), 1);
        assert!(s.refresh_community_with(11 * DAY + 1, false, get).unwrap());
        assert_eq!(calls.get(), 2);
        assert_eq!(s.active_rules().len(), 1);
        s.apply(MemoryOp::AddRule { rule: rule("~/.gradle/caches", "My gradle") }).unwrap();
        assert_eq!(s.active_rules().len(), 1, "local rule shadows the community one");
        s.apply(MemoryOp::Community { enabled: false }).unwrap();
        assert!(!s.refresh_community_with(20 * DAY, true, |_| panic!("disabled")).unwrap());
        assert_eq!(s.memory().community.rules, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    impl Store {
        fn submit_outbox_test(&mut self) -> SubmitReport {
            share::submit_with(&self.dir, None, |_, _| panic!("no endpoint"))
        }
    }
}
