//! Claude Code (~/.claude). Sessions are `projects/<slug>/<id>.jsonl` plus the per-session
//! folders Claude keeps next to it and elsewhere: `projects/<slug>/<id>/` (subagents, tool
//! results), `file-history/<id>`, `todos/<id>-*.json`, `debug/<id>.txt`, `session-env/<id>`,
//! `tasks/<id>`. Project-local worktrees (`<repo>/.claude/worktrees/*`) count as sessions too.
//! Title: the last custom title, else AI title, else summary, else the first user message.
//! A session listed in `sessions/<pid>.json` with a live pid is running.

use super::{git, t3::Names, time_of, Found, Raw, SessionEnv};
use crate::util::{self, clean_title, name_of};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PEEK: usize = 96 * 1024;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Meta {
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub started: Option<i64>,
    pub ended: Option<i64>,
}

/// Title and cwd from the first and last few KB of a session file.
pub(crate) fn read_meta(path: &Path) -> Meta {
    let head = util::head(path, PEEK);
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let tail = if len as usize > PEEK { util::tail(path, PEEK as u64) } else { Vec::new() };
    let mut m = Meta::default();
    let (mut custom, mut ai, mut summary, mut first_user) = (None, None, None, None);
    for (i, buf) in [&head, &tail].into_iter().enumerate() {
        for v in util::json_lines(buf) {
            if m.cwd.is_none() {
                m.cwd = v["cwd"].as_str().map(str::to_string);
            }
            if let Some(t) = time_of(&v["timestamp"]) {
                if i == 0 && m.started.is_none() {
                    m.started = Some(t);
                }
                m.ended = Some(m.ended.map_or(t, |e: i64| e.max(t)));
            }
            match v["type"].as_str() {
                Some("custom-title") => custom = v["customTitle"].as_str().map(str::to_string).or(custom),
                Some("ai-title") => ai = v["aiTitle"].as_str().map(str::to_string).or(ai),
                Some("summary") => summary = v["summary"].as_str().map(str::to_string).or(summary),
                Some("user") if first_user.is_none() && v["isMeta"] != true && v["isSidechain"] != true => {
                    let c = &v["message"]["content"];
                    let text = c.as_str().map(str::to_string).or_else(|| {
                        c.as_array().and_then(|a| a.iter().find(|x| x["type"] == "text").and_then(|x| x["text"].as_str()).map(str::to_string))
                    });
                    first_user = text.filter(|t| !t.trim_start().starts_with('<'));
                }
                _ => {}
            }
        }
    }
    m.title = [custom, ai, summary, first_user].into_iter().flatten().find_map(|t| clean_title(&t, 120));
    m
}

/// "-Users-me-Documents-my-app" → "/Users/me/Documents/my-app", checking the disk to put the
/// dashes back; "--" stands for "/." (hidden folder).
pub(crate) fn decode_slug(slug: &str) -> PathBuf {
    let tokens: Vec<String> = {
        let mut out = Vec::new();
        let mut dot = false;
        for t in slug.trim_start_matches('-').split('-') {
            if t.is_empty() {
                dot = true;
                continue;
            }
            out.push(if dot { format!(".{}", t) } else { t.to_string() });
            dot = false;
        }
        out
    };
    fn go(base: &Path, rest: &[String]) -> Option<PathBuf> {
        if rest.is_empty() {
            return Some(base.to_path_buf());
        }
        for take in (1..=rest.len()).rev() {
            let name = rest[..take].join("-");
            let p = base.join(&name);
            if p.is_dir() {
                if let Some(found) = go(&p, &rest[take..]) {
                    return Some(found);
                }
            }
        }
        None
    }
    go(Path::new("/"), &tokens).unwrap_or_else(|| PathBuf::from(format!("/{}", tokens.join("/"))))
}

fn running_sessions(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for p in util::children(&root.join("sessions")) {
        if !name_of(&p).ends_with(".json") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
        let (Some(pid), Some(id)) = (v["pid"].as_i64(), v["sessionId"].as_str()) else { continue };
        if util::pid_alive(pid as i32) {
            out.insert(id.to_string(), "Running now".to_string());
        }
    }
    out
}

pub fn find(env: &SessionEnv, names: &Names) -> Found {
    let root = env.home.join(".claude");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    if !root.is_dir() {
        return found;
    }
    let running = running_sessions(&root);
    // Per-session files kept outside projects/, indexed by session id.
    let mut extra: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for (dir, strip) in [("file-history", ""), ("debug", ".txt"), ("session-env", ""), ("tasks", ""), ("todos", "")] {
        for p in util::children(&root.join(dir)) {
            let n = name_of(&p);
            let id = if dir == "todos" { n.get(..36).unwrap_or("").to_string() } else { n.strip_suffix(strip).unwrap_or(&n).to_string() };
            if id.len() == 36 {
                extra.entry(id).or_default().push(p);
            }
        }
    }

    let project_dirs = util::subdirs(&root.join("projects"));
    let per_project: Vec<(Vec<Raw>, Option<String>)> = project_dirs
        .par_iter()
        .map(|pd| {
            let mut raws = Vec::new();
            let mut cwd: Option<String> = None;
            let entries = util::children(pd);
            for p in &entries {
                let n = name_of(p);
                let Some(id) = n.strip_suffix(".jsonl") else { continue };
                let meta = read_meta(p);
                if cwd.is_none() {
                    cwd = meta.cwd.clone();
                }
                let mut paths = vec![p.clone()];
                let dir = pd.join(id);
                if dir.is_dir() {
                    paths.push(dir);
                }
                raws.push(Raw {
                    id: id.to_string(),
                    title: meta.title,
                    project: meta.cwd.map(PathBuf::from),
                    started: meta.started,
                    last_used: meta.ended,
                    paths,
                    kind: "session",
                    ..Default::default()
                });
            }
            // Session folders whose transcript is gone.
            for p in &entries {
                let n = name_of(p);
                if n.len() == 36 && p.is_dir() && !pd.join(format!("{}.jsonl", n)).exists() {
                    raws.push(Raw { id: n.clone(), title: None, paths: vec![p.clone()], kind: "session", ..Default::default() });
                }
            }
            (raws, cwd)
        })
        .collect();

    let mut project_paths: Vec<PathBuf> = Vec::new();
    for ((mut raws, cwd), pd) in per_project.into_iter().zip(project_dirs.iter()) {
        let project = cwd.map(PathBuf::from).unwrap_or_else(|| decode_slug(&name_of(pd)));
        project_paths.push(project.clone());
        for r in raws.iter_mut() {
            if r.project.is_none() {
                r.project = Some(project.clone());
            }
            if let Some(e) = extra.remove(&r.id) {
                r.paths.extend(e);
            }
            r.busy = running.get(&r.id).cloned().or_else(|| r.last_used.filter(|t| env.now - t < 120).map(|_| "Active a moment ago".into()));
            if r.title.is_none() {
                r.title = r.project.as_ref().and_then(|p| names.folders.get(p).cloned());
            }
        }
        found.sessions.extend(raws);
    }
    // Leftover per-session files of sessions whose transcript is gone.
    let leftovers: Vec<PathBuf> = extra.into_values().flatten().collect();
    if !leftovers.is_empty() {
        found.sessions.push(Raw {
            id: "claude:leftovers".into(),
            title: Some("Files of deleted sessions".into()),
            project_name: Some("Left behind".into()),
            paths: leftovers,
            kind: "session",
            ..Default::default()
        });
    }

    // Project-local worktrees.
    project_paths.sort();
    project_paths.dedup();
    for pp in project_paths {
        for wt in util::subdirs(&pp.join(".claude/worktrees")) {
            found.sessions.push(Raw {
                id: crate::util::s(&wt),
                title: Some(name_of(&wt)),
                project: Some(pp.clone()),
                last_used: crate::size::own_mtime(&wt),
                warning: if env.git { git::warning(&wt) } else { None },
                paths: vec![wt],
                kind: "worktree",
                ..Default::default()
            });
        }
    }

    found.shared = vec![
        ("Plugins".into(), root.join("plugins"), false, "Installed Claude Code plugins. Manage them with /plugin.".into()),
        ("Skills".into(), root.join("skills"), false, "Your skills. Kept.".into()),
        ("Shell snapshots".into(), root.join("shell-snapshots"), true, "Shell environment captures. Regrow.".into()),
        ("Backups".into(), root.join("backups"), false, "Claude Code's own config backups.".into()),
        ("Cache".into(), root.join("cache"), true, "Regrows.".into()),
    ];
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sessions_titles_extra_files_and_projects() {
        let home = util::test_dir("claude");
        let repo = home.join("Code/my-app");
        fs::create_dir_all(repo.join(".claude/worktrees/feature-x")).unwrap();
        let root = home.join(".claude");
        let slug = format!("{}", crate::util::s(&repo).replace('/', "-"));
        let pd = root.join("projects").join(&slug);
        fs::create_dir_all(pd.join("memory")).unwrap();
        fs::write(pd.join("memory/MEMORY.md"), b"keep me").unwrap();
        let a = "0342e0ba-7424-4a08-82b2-0f8a6abb8786";
        let b = "11111111-2222-3333-4444-555555555555";
        let cwd = crate::util::s(&repo);
        let lines_a = [
            serde_json::json!({"type":"custom-title","customTitle":"Creator finder","sessionId":a}),
            serde_json::json!({"type":"user","cwd":cwd,"timestamp":"2026-11-09T15:59:35.741Z","message":{"role":"user","content":"Let's find creators"}}),
            serde_json::json!({"type":"assistant","cwd":cwd,"timestamp":"2026-11-09T16:30:00.000Z","message":{"content":[]}}),
        ];
        fs::write(pd.join(format!("{a}.jsonl")), lines_a.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        fs::create_dir_all(pd.join(a).join("subagents")).unwrap();
        fs::write(pd.join(a).join("subagents/agent-1.jsonl"), vec![1u8; 9000]).unwrap();
        let lines_b = [
            serde_json::json!({"type":"user","isMeta":true,"cwd":cwd,"timestamp":"2026-11-01T10:00:00Z","message":{"content":"<command-name>/model</command-name>"}}),
            serde_json::json!({"type":"user","cwd":cwd,"timestamp":"2026-11-01T10:00:01Z","message":{"content":[{"type":"text","text":"# Fix the login bug\nplease"}]}}),
        ];
        fs::write(pd.join(format!("{b}.jsonl")), lines_b.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        fs::create_dir_all(root.join("file-history").join(a)).unwrap();
        fs::create_dir_all(root.join("todos")).unwrap();
        fs::write(root.join(format!("todos/{a}-agent-{a}.json")), b"[]").unwrap();
        fs::create_dir_all(root.join("debug")).unwrap();
        fs::write(root.join("debug/99999999-9999-9999-9999-999999999999.txt"), b"old").unwrap();
        fs::write(root.join("settings.json"), b"{}").unwrap();

        let env = SessionEnv { git: false, ..SessionEnv::fixture(&home) };
        let f = find(&env, &Names::default());
        let sa = f.sessions.iter().find(|s| s.id == a).unwrap();
        assert_eq!(sa.title.as_deref(), Some("Creator finder"));
        assert_eq!(sa.project.as_deref(), Some(repo.as_path()));
        assert_eq!(sa.started, util::parse_time("2026-11-09T15:59:35.741Z"));
        assert_eq!(sa.last_used, util::parse_time("2026-11-09T16:30:00Z"));
        assert_eq!(sa.paths.len(), 4); // jsonl, dir, file-history, todos
        let sb = f.sessions.iter().find(|s| s.id == b).unwrap();
        assert_eq!(sb.title.as_deref(), Some("Fix the login bug"));
        assert!(f.sessions.iter().any(|s| s.id == "claude:leftovers"));
        assert!(f.sessions.iter().any(|s| s.kind == "worktree" && s.title.as_deref() == Some("feature-x")));
        // Memory never shows up in a session.
        assert!(!f.sessions.iter().flat_map(|s| s.paths.iter()).any(|p| p.to_string_lossy().contains("memory")));

        let tool = super::super::inventory(&env).into_iter().find(|t| t.tool == "claude-code").unwrap();
        let proj = tool.projects.iter().find(|p| p.name == "my-app").unwrap();
        assert!(proj.sessions.len() >= 3);
        // Worktree lives outside ~/.claude, so it's added on top of the root's du.
        assert_eq!(tool.session_bytes + tool.shared.iter().map(|s| s.bytes).sum::<u64>(), tool.bytes);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn slug_decoding_restores_dashes_and_dots() {
        let home = util::test_dir("slug");
        let p = home.join("Documents/my-app/.worktrees/x");
        fs::create_dir_all(&p).unwrap();
        let slug = crate::util::s(&p).replace("/.", "--").replace('/', "-");
        assert_eq!(decode_slug(&slug), p);
        assert_eq!(decode_slug("-nope-a-b"), PathBuf::from("/nope/a/b"));
        fs::remove_dir_all(&home).unwrap();
    }
}
