//! T3 Code (~/.t3). Threads come from its SQLite projection (opened read-only); a thread's disk
//! space is its scratch folder or worktree under ~/.t3 and its attachments
//! (`userdata/attachments/<thread id>-<uuid>.<ext>`). The chat history itself lives in
//! `userdata/statev2.sqlite`, shared by all threads: deleting threads inside T3 shrinks it, Disko
//! never edits it. Worktrees and scratch folders no thread points at are listed on their own.

use super::{git, Found, Raw, SessionEnv};
use crate::util::{self, name_of, parse_time};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Titles T3 knows, used by every adapter to name projects and scratch folders.
#[derive(Debug, Default, Clone)]
pub struct Names {
    /// workspace root → project title
    pub projects: HashMap<PathBuf, String>,
    /// scratch / worktree folder → thread title
    pub folders: HashMap<PathBuf, String>,
}

impl Names {
    pub fn project_title(&self, p: &Path) -> Option<String> {
        if let Some(t) = self.projects.get(p) {
            return Some(t.clone());
        }
        // A worktree: name it after its main repository.
        git::main_repo(p).filter(|r| r != p).and_then(|r| self.projects.get(&r).cloned().or_else(|| Some(name_of(&r))))
    }
}

struct Thread {
    id: String,
    project: Option<String>,
    title: String,
    created: Option<i64>,
    updated: Option<i64>,
    worktree: Option<PathBuf>,
    deleted: bool,
}

struct Db {
    projects: HashMap<String, (String, PathBuf)>,
    threads: Vec<Thread>,
    running: std::collections::HashSet<String>,
}

fn db_path(home: &Path) -> Option<PathBuf> {
    let base = home.join(".t3/userdata");
    [base.join("statev2.sqlite"), base.join("state.sqlite")].into_iter().find(|p| p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
}

fn load(home: &Path, now: i64) -> Option<Db> {
    let c = util::open_ro(&db_path(home)?)?;
    let mut projects = HashMap::new();
    if let Ok(mut st) = c.prepare("SELECT project_id, title, workspace_root FROM projection_projects") {
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)));
        for (id, title, root) in rows.into_iter().flatten().flatten() {
            projects.insert(id, (title, PathBuf::from(root)));
        }
    }
    let mut threads = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let v2 = "SELECT thread_id, project_id, title, created_at, updated_at, deleted_at, json_extract(payload_json,'$.worktreePath') FROM orchestration_v2_projection_threads";
    let v1 = "SELECT thread_id, project_id, title, created_at, updated_at, deleted_at, worktree_path FROM projection_threads";
    for sql in [v2, v1] {
        let Ok(mut st) = c.prepare(sql) else { continue };
        let rows = st.query_map([], |r| {
            Ok(Thread {
                id: r.get(0)?,
                project: r.get(1).ok(),
                title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                created: r.get::<_, Option<String>>(3)?.and_then(|t| parse_time(&t)),
                updated: r.get::<_, Option<String>>(4)?.and_then(|t| parse_time(&t)),
                deleted: r.get::<_, Option<String>>(5)?.is_some(),
                worktree: r.get::<_, Option<String>>(6)?.filter(|s| !s.is_empty()).map(PathBuf::from),
            })
        });
        for t in rows.into_iter().flatten().flatten() {
            if seen.insert(t.id.clone()) {
                threads.push(t);
            }
        }
    }
    let mut running = std::collections::HashSet::new();
    if let Ok(mut st) = c.prepare("SELECT thread_id, requested_at FROM orchestration_v2_projection_runs WHERE status IN ('queued','preparing','starting','running','waiting')") {
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
        for (id, at) in rows.into_iter().flatten().flatten() {
            // A run "running" for half a day is a stale record from a crash, not live work.
            if parse_time(&at).is_some_and(|t| now - t < 12 * 3600) {
                running.insert(id);
            }
        }
    }
    Some(Db { projects, threads, running })
}

/// Project and folder titles for the other adapters.
pub fn names(env: &SessionEnv) -> Names {
    let mut n = Names::default();
    let Some(db) = load(&env.home, env.now) else { return n };
    for (title, root) in db.projects.values() {
        n.projects.insert(root.clone(), title.clone());
    }
    let mut ts: Vec<&Thread> = db.threads.iter().filter(|t| t.worktree.is_some() && !t.title.is_empty()).collect();
    ts.sort_by_key(|t| (is_child_thread(&t.id), t.created));
    for t in ts {
        n.folders.entry(t.worktree.clone().unwrap()).or_insert_with(|| t.title.clone());
    }
    n
}

/// "thread:delegated-task:…" and provider-native threads are children of a user thread.
fn is_child_thread(id: &str) -> bool {
    id.starts_with("thread:")
}

pub fn find(env: &SessionEnv) -> Found {
    let home = &env.home;
    let root = home.join(".t3");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    if !root.is_dir() {
        return found;
    }
    let scratch = root.join("scratch");
    let worktrees = root.join("worktrees");
    let attachments = root.join("userdata/attachments");
    let db = load(home, env.now);

    // Attachments by thread id (first 36 chars of the file name).
    let mut attach: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for p in util::children(&attachments) {
        let n = name_of(&p);
        if n.len() > 37 && n.as_bytes()[36] == b'-' {
            attach.entry(n[..36].to_string()).or_default().push(p);
        }
    }

    // Folders T3 owns: scratch/<dir>, worktrees/<repo>/<dir>.
    let mut folders: Vec<(PathBuf, &'static str)> = util::subdirs(&scratch).into_iter().map(|p| (p, "scratch")).collect();
    for repo in util::subdirs(&worktrees) {
        folders.extend(util::subdirs(&repo).into_iter().map(|p| (p, "worktree")));
    }
    let mut folder_owner: HashMap<PathBuf, usize> = HashMap::new();
    let mut sessions: Vec<Raw> = Vec::new();

    if let Some(db) = &db {
        // Top-level threads first (oldest first), so a shared folder belongs to the thread
        // that created it rather than to one of its delegated children.
        let mut order: Vec<&Thread> = db.threads.iter().collect();
        order.sort_by_key(|t| (is_child_thread(&t.id), t.created));
        for t in order {
            let mut paths = Vec::new();
            let mut kind = "session";
            if let Some(w) = &t.worktree {
                if (w.starts_with(&scratch) || w.starts_with(&worktrees)) && w.is_dir() && !folder_owner.contains_key(w) {
                    // Only whole folders T3 created, never the user's own repo.
                    if folders.iter().any(|(f, _)| f == w) {
                        folder_owner.insert(w.clone(), sessions.len());
                        kind = if w.starts_with(&scratch) { "scratch" } else { "worktree" };
                        paths.push(w.clone());
                    }
                }
            }
            if let Some(a) = attach.remove(&t.id) {
                paths.extend(a);
            }
            if paths.is_empty() {
                continue;
            }
            let (project, project_name) = match t.project.as_ref().and_then(|p| db.projects.get(p)) {
                Some((title, root)) => (Some(root.clone()), Some(title.clone())),
                None => (t.worktree.clone(), None),
            };
            let busy = db.running.contains(&t.id).then(|| "Running in T3 Code".to_string());
            let warning = if kind == "worktree" && env.git { git::warning(&paths[0]) } else { None };
            let warning = match (warning, t.deleted) {
                (w, false) => w,
                (Some(w), true) => Some(format!("Thread was deleted in T3 · {}", w)),
                (None, true) => Some("Thread was deleted in T3; these files were left behind".into()),
            };
            sessions.push(Raw {
                id: t.id.clone(),
                title: Some(t.title.clone()).filter(|x| !x.is_empty()),
                project,
                project_name,
                started: t.created,
                last_used: t.updated,
                paths,
                kind,
                busy,
                warning,
            });
        }
    }

    // Folders no thread claims.
    for (f, kind) in folders {
        if folder_owner.contains_key(&f) {
            continue;
        }
        let project = if kind == "worktree" { git::main_repo(&f).or_else(|| Some(f.clone())) } else { Some(f.clone()) };
        let warning = if kind == "worktree" && env.git { git::warning(&f) } else { None };
        sessions.push(Raw {
            id: crate::util::s(&f),
            title: Some(name_of(&f)),
            project,
            project_name: None,
            started: None,
            last_used: None,
            paths: vec![f],
            kind,
            busy: None,
            warning: Some(match warning {
                Some(w) => format!("No T3 thread uses this folder · {}", w),
                None => "No T3 thread uses this folder".into(),
            }),
        });
    }
    // Attachments of threads T3 no longer knows.
    let orphans: Vec<PathBuf> = attach.into_values().flatten().collect();
    if !orphans.is_empty() {
        sessions.push(Raw {
            id: "t3:orphan-attachments".into(),
            title: Some("Attachments of deleted threads".into()),
            project_name: Some("Left behind".into()),
            paths: orphans,
            kind: "session",
            ..Default::default()
        });
    }
    // Recent activity marks a session busy even without a run record.
    for s in sessions.iter_mut() {
        if s.busy.is_none() && s.last_used.is_some_and(|t| env.now - t < 120) {
            s.busy = Some("Active a moment ago".into());
        }
    }
    found.sessions = sessions;

    let u = root.join("userdata");
    let keep = "Managed by T3 Code.";
    found.shared = vec![
        ("Chat history database".into(), u.join("statev2.sqlite"), false, "Every T3 thread's messages and events. Delete or archive threads in T3 Code to shrink it.".into()),
        ("Chat database journal".into(), u.join("statev2.sqlite-wal"), false, keep.into()),
        ("Old chat database (before T3's v2 migration)".into(), u.join("state.sqlite"), false, "T3 imported it into the new database. Remove it from T3's settings if it offers that.".into()),
        ("Logs".into(), u.join("logs"), true, "T3 Code's trace logs. Regrow.".into()),
        ("Browser screenshots".into(), u.join("browser-artifacts"), true, "Screenshots taken by agents in T3's browser.".into()),
        ("Browser profile".into(), u.join("server-browser"), false, keep.into()),
        ("Usage caches".into(), u.join("usage-scan-cache-v11.json"), true, "Rebuilt by T3 when needed.".into()),
        ("Old usage cache".into(), u.join("usage-scan-cache-v9.json"), true, "Superseded by a newer cache version.".into()),
        ("Old usage cache (v1)".into(), u.join("usage-scan-cache.json"), true, "Superseded by a newer cache version.".into()),
        ("Bundled tools".into(), root.join("tools"), false, "Tools T3 Code downloaded for its agents (Codex, Chrome, device tools).".into()),
    ];
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn threads_folders_attachments_and_orphans() {
        let home = util::test_dir("t3");
        let t3 = home.join(".t3");
        let ud = t3.join("userdata");
        std::fs::create_dir_all(ud.join("attachments")).unwrap();
        let scratch = t3.join("scratch/2026-11-01-fix-the-thing-aaaa1111");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("notes.md"), vec![1u8; 10_000]).unwrap();
        let orphan = t3.join("scratch/2026-10-01-forgotten-bbbb2222");
        std::fs::create_dir_all(&orphan).unwrap();
        let tid = "14897c6f-46e7-4812-aea6-06143cf2f23f";
        std::fs::write(ud.join(format!("attachments/{tid}-00000000-0000-0000-0000-000000000000.png")), vec![1u8; 5000]).unwrap();
        std::fs::write(ud.join("attachments/99999999-9999-9999-9999-999999999999-11111111-0000-0000-0000-000000000000.txt"), b"x").unwrap();
        let c = Connection::open(ud.join("statev2.sqlite")).unwrap();
        c.execute_batch(
            "CREATE TABLE projection_projects(project_id TEXT, title TEXT, workspace_root TEXT);
             CREATE TABLE orchestration_v2_projection_threads(thread_id TEXT, project_id TEXT, title TEXT, created_at TEXT, updated_at TEXT, deleted_at TEXT, payload_json TEXT);
             CREATE TABLE orchestration_v2_projection_runs(thread_id TEXT, status TEXT, requested_at TEXT);",
        )
        .unwrap();
        c.execute("INSERT INTO projection_projects VALUES('p','No project',?1)", [crate::util::s(&t3.join("scratch"))]).unwrap();
        let payload = serde_json::json!({ "worktreePath": scratch }).to_string();
        c.execute("INSERT INTO orchestration_v2_projection_threads VALUES(?1,'p','Fix the thing','2026-11-01T10:00:00Z','2026-11-02T10:00:00Z',NULL,?2)", [tid, &payload]).unwrap();
        // A delegated child pointing at the same folder must not steal it.
        c.execute("INSERT INTO orchestration_v2_projection_threads VALUES('thread:delegated-task:x','p','Child','2026-10-01T10:00:00Z','2026-11-02T10:00:00Z',NULL,?1)", [&payload]).unwrap();
        drop(c);

        let env = SessionEnv { git: false, ..SessionEnv::fixture(&home) };
        let names = names(&env);
        assert_eq!(names.folders.get(&scratch).map(String::as_str), Some("Fix the thing"));
        let f = find(&env);
        let main = f.sessions.iter().find(|s| s.id == tid).unwrap();
        assert_eq!(main.title.as_deref(), Some("Fix the thing"));
        assert_eq!(main.kind, "scratch");
        assert_eq!(main.paths.len(), 2);
        assert_eq!(main.started, parse_time("2026-11-01T10:00:00Z"));
        assert!(!f.sessions.iter().any(|s| s.id.starts_with("thread:")));
        let o = f.sessions.iter().find(|s| s.paths == vec![orphan.clone()]).unwrap();
        assert!(o.warning.as_deref().unwrap().contains("No T3 thread"));
        assert!(f.sessions.iter().any(|s| s.id == "t3:orphan-attachments"));

        let tool = super::super::inventory(&env).into_iter().find(|t| t.tool == "t3code").unwrap();
        assert_eq!(tool.projects.iter().find(|p| p.name == "Scratch chats").unwrap().sessions.len(), 2);
        assert_eq!(tool.session_bytes + tool.shared.iter().map(|s| s.bytes).sum::<u64>(), tool.bytes);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
