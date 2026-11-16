//! Codex (~/.codex). Sessions are rollout files `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`
//! (and `archived_sessions/`, sometimes `.jsonl.zst`), plus `generated_images/<id>/` and
//! `attachments/<id>/`. Titles and working folders come from the newest `state_N.sqlite`
//! (threads table, read-only) and `session_index.jsonl`; files the database doesn't know are
//! read directly (the `session_meta` first line has the cwd). `worktrees/<name>/` are Codex's
//! own checkouts. `thread_history_N.sqlite` is a shared index of all threads, rebuilt by Codex.

use super::{git, t3::Names, Found, Raw, SessionEnv};
use crate::util::{self, clean_title, name_of};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Row {
    cwd: Option<String>,
    title: Option<String>,
    created: Option<i64>,
    updated: Option<i64>,
}

fn newest_db(root: &Path, prefix: &str) -> Option<PathBuf> {
    let mut v: Vec<(u32, PathBuf)> = util::children(root)
        .into_iter()
        .filter_map(|p| {
            let n = name_of(&p);
            let num = n.strip_prefix(prefix)?.strip_suffix(".sqlite")?.parse().ok()?;
            Some((num, p))
        })
        .collect();
    v.sort();
    v.pop().map(|(_, p)| p)
}

fn load_rows(root: &Path) -> HashMap<String, Row> {
    let mut out = HashMap::new();
    let Some(c) = newest_db(root, "state_").and_then(|p| util::open_ro(&p)) else { return out };
    let Ok(mut st) = c.prepare("SELECT id, cwd, title, first_user_message, created_at, updated_at FROM threads") else { return out };
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3).unwrap_or(None),
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<i64>>(5)?,
        ))
    });
    for (id, cwd, title, first, created, updated) in rows.into_iter().flatten().flatten() {
        let title = title.filter(|t| !t.trim().is_empty()).or(first);
        out.insert(id, Row { cwd: cwd.filter(|c| !c.is_empty()), title, created: created.map(util::secs), updated: updated.map(util::secs) });
    }
    out
}

fn index_titles(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(t) = std::fs::read(root.join("session_index.jsonl")) else { return out };
    for v in util::json_lines(&t) {
        if let (Some(id), Some(name)) = (v["id"].as_str(), v["thread_name"].as_str()) {
            out.insert(id.to_string(), name.to_string());
        }
    }
    out
}

/// Session id from "rollout-2026-11-03T23-31-51-<uuid>.jsonl[.zst]".
pub(crate) fn rollout_id(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".zst").unwrap_or(name);
    let stem = stem.strip_suffix(".jsonl")?;
    if !stem.starts_with("rollout-") || stem.len() < 36 {
        return None;
    }
    let id = &stem[stem.len() - 36..];
    (id.as_bytes()[8] == b'-' && id.as_bytes()[13] == b'-').then(|| id.to_string())
}

fn rollouts(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    for p in util::children(dir) {
        if p.is_dir() {
            if depth > 0 {
                rollouts(&p, depth - 1, out);
            }
        } else if rollout_id(&name_of(&p)).is_some() {
            out.push(p);
        }
    }
}

pub fn find(env: &SessionEnv, names: &Names) -> Found {
    let root = env.home.join(".codex");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    if !root.is_dir() {
        return found;
    }
    let rows = load_rows(&root);
    let titles = index_titles(&root);
    let mut files = Vec::new();
    rollouts(&root.join("sessions"), 3, &mut files);
    rollouts(&root.join("archived_sessions"), 3, &mut files);

    let mut by_id: HashMap<String, Raw> = HashMap::new();
    for f in files {
        let id = rollout_id(&name_of(&f)).unwrap();
        let archived = f.starts_with(root.join("archived_sessions"));
        let e = by_id.entry(id.clone()).or_insert_with(|| {
            let row = rows.get(&id);
            let mut cwd = row.and_then(|r| r.cwd.clone());
            if cwd.is_none() && !name_of(&f).ends_with(".zst") {
                cwd = util::raw_string_field(&util::head(&f, 256 * 1024), "cwd");
            }
            let title = titles.get(&id).cloned().or_else(|| row.and_then(|r| r.title.clone())).and_then(|t| clean_title(&t, 120));
            Raw {
                id: id.clone(),
                title,
                project: cwd.map(PathBuf::from),
                started: row.and_then(|r| r.created),
                last_used: row.and_then(|r| r.updated),
                kind: "session",
                warning: archived.then(|| "Archived in Codex".to_string()),
                ..Default::default()
            }
        });
        e.paths.push(f);
    }
    for dir in ["generated_images", "attachments"] {
        for p in util::subdirs(&root.join(dir)) {
            if let Some(r) = by_id.get_mut(&name_of(&p)) {
                r.paths.push(p);
            }
        }
    }
    for r in by_id.values_mut() {
        if r.title.is_none() {
            r.title = r.project.as_ref().and_then(|p| names.folders.get(p).cloned());
        }
        if r.last_used.is_some_and(|t| env.now - t < 120) {
            r.busy = Some("Active a moment ago".into());
        }
    }
    found.sessions = by_id.into_values().collect();

    // Codex's own worktrees: worktrees/<name>/<repo checkout>.
    for wt in util::subdirs(&root.join("worktrees")) {
        let checkout = util::subdirs(&wt).into_iter().find(|c| util::exists(&c.join(".git")));
        let repo = checkout.as_ref().and_then(|c| git::main_repo(c));
        let warning = match (&checkout, env.git) {
            (Some(c), true) => git::warning(c),
            _ => None,
        };
        found.sessions.push(Raw {
            id: crate::util::s(&wt),
            title: Some(name_of(&wt)),
            project: repo.or_else(|| checkout.clone()),
            last_used: crate::size::own_mtime(&wt),
            paths: vec![wt],
            kind: "worktree",
            warning,
            ..Default::default()
        });
    }

    let mut shared: Vec<(String, PathBuf, bool, String)> = Vec::new();
    if let Some(p) = newest_db(&root, "thread_history_") {
        shared.push(("Thread history index".into(), p, false, "Codex's searchable copy of all threads. Archive threads in Codex to shrink it.".into()));
    }
    if let Some(p) = newest_db(&root, "logs_") {
        shared.push(("Log database".into(), p, false, "Codex's logs. Managed by Codex.".into()));
    }
    if let Some(p) = newest_db(&root, "state_") {
        shared.push(("Thread list database".into(), p, false, "Codex's list of threads. Managed by Codex.".into()));
    }
    shared.extend([
        ("Packages".into(), root.join("packages"), false, "Runtimes Codex installed for its tools.".into()),
        ("Plugins".into(), root.join("plugins"), false, "Installed Codex plugins.".into()),
        ("Cache".into(), root.join("cache"), true, "Regrows.".into()),
        ("Temporary files".into(), root.join(".tmp"), true, "Codex's scratch space between runs.".into()),
        ("Computer use".into(), root.join("computer-use"), false, "Codex's computer-use helper.".into()),
    ]);
    found.shared = shared;
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rollouts_titles_images_and_worktrees() {
        let home = util::test_dir("codex");
        let root = home.join(".codex");
        let day = root.join("sessions/2026/11/03");
        fs::create_dir_all(&day).unwrap();
        let a = "01a1a38a-49bb-7ab2-850d-40e5c3e1dcfd";
        let b = "01a1a185-992e-72c3-8e68-db19db5e3411";
        let meta_b = serde_json::json!({"timestamp":"2026-11-03T22:31:54.976Z","type":"session_meta","payload":{"id":b,"cwd":"/Users/x/Proj","base_instructions":{"text":"long"}}});
        fs::write(day.join(format!("rollout-2026-11-03T23-31-51-{a}.jsonl")), vec![b'x'; 30_000]).unwrap();
        fs::write(day.join(format!("rollout-2026-11-03T14-07-29-{b}.jsonl")), meta_b.to_string()).unwrap();
        fs::create_dir_all(root.join("archived_sessions")).unwrap();
        fs::write(root.join("archived_sessions/rollout-2026-09-23T13-47-27-01a0ce17-951e-7fd1-b090-a5f0bd2bc1b1.jsonl.zst"), b"z").unwrap();
        fs::create_dir_all(root.join("generated_images").join(a)).unwrap();
        fs::write(root.join("generated_images").join(a).join("img.png"), vec![1u8; 4000]).unwrap();
        fs::write(root.join("session_index.jsonl"), serde_json::json!({"id":a,"thread_name":"Update models"}).to_string() + "\n").unwrap();
        fs::write(root.join("auth.json"), b"{}").unwrap();
        let c = rusqlite::Connection::open(root.join("state_5.sqlite")).unwrap();
        c.execute_batch("CREATE TABLE threads(id TEXT, cwd TEXT, title TEXT, first_user_message TEXT, created_at INTEGER, updated_at INTEGER);").unwrap();
        c.execute("INSERT INTO threads VALUES(?1,'/Users/x/Other','db title','hi',1794824809,1794830606)", [a]).unwrap();
        drop(c);
        fs::create_dir_all(root.join("worktrees/followup/Repo")).unwrap();

        let env = SessionEnv { git: false, ..SessionEnv::fixture(&home) };
        let f = find(&env, &Names::default());
        let sa = f.sessions.iter().find(|s| s.id == a).unwrap();
        assert_eq!(sa.title.as_deref(), Some("Update models"));
        assert_eq!(sa.project.as_deref(), Some(Path::new("/Users/x/Other")));
        assert_eq!(sa.started, Some(1794824809));
        assert_eq!(sa.paths.len(), 2);
        let sb = f.sessions.iter().find(|s| s.id == b).unwrap();
        assert_eq!(sb.project.as_deref(), Some(Path::new("/Users/x/Proj")));
        assert!(f.sessions.iter().any(|s| s.warning.as_deref() == Some("Archived in Codex")));
        assert!(f.sessions.iter().any(|s| s.kind == "worktree" && s.title.as_deref() == Some("followup")));
        assert!(!f.sessions.iter().flat_map(|s| &s.paths).any(|p| p.ends_with("auth.json")));
        assert_eq!(rollout_id("rollout-x.jsonl"), None);
        fs::remove_dir_all(&home).unwrap();
    }
}
