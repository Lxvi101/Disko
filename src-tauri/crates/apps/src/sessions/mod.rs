//! Coding-agent sessions: for every agent, which project and which session/thread takes how much
//! space, since when, and what deleting it would move to the Trash.
//!
//! Each adapter reads the tool's own metadata read-only (JSONL heads/tails, SQLite opened
//! read-only) and returns raw sessions with the paths that belong to exactly that session.
//! The builder measures them in parallel, groups them by project and adds the tool's shared
//! pieces (its database, logs, packages, settings) so the totals add up to what's on disk.
//!
//! Never part of a session: credentials, settings, config, MCP configs, plugins, skills,
//! CLAUDE.md / AGENTS.md, memory folders (`guard::never_delete`).

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod gemini;
pub mod git;
pub mod guard;
pub mod opencode;
pub mod t3;

use crate::size::{self, max_opt, Usage};
use crate::types::{AgentProject, AgentSession, AgentShared, AgentTool};
use crate::util::{self, name_of, s};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Where to look. `SessionEnv::real()` for this Mac.
#[derive(Debug, Clone)]
pub struct SessionEnv {
    pub home: PathBuf,
    /// Run git on worktrees for uncommitted/unpushed warnings.
    pub git: bool,
    pub now: i64,
}

impl SessionEnv {
    pub fn real() -> Self {
        SessionEnv { home: disko_core::paths::home(), git: true, now: util::now() }
    }
    pub fn fixture(home: &Path) -> Self {
        SessionEnv { home: home.to_path_buf(), git: true, now: util::now() }
    }
}

/// A session as an adapter found it, before measuring and grouping.
#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub id: String,
    pub title: Option<String>,
    /// Working directory / repository the session is about.
    pub project: Option<PathBuf>,
    /// Display name when the tool knows a better one than the folder name.
    pub project_name: Option<String>,
    pub started: Option<i64>,
    pub last_used: Option<i64>,
    pub paths: Vec<PathBuf>,
    pub kind: &'static str,
    pub busy: Option<String>,
    pub warning: Option<String>,
}

/// What an adapter returns.
#[derive(Debug, Default)]
pub struct Found {
    pub roots: Vec<PathBuf>,
    pub sessions: Vec<Raw>,
    /// Named shared pieces: (label, path, removable, why).
    pub shared: Vec<(String, PathBuf, bool, String)>,
}

/// Tools with something on disk, largest first.
pub fn sessions() -> Vec<AgentTool> {
    inventory(&SessionEnv::real())
}

pub fn inventory(env: &SessionEnv) -> Vec<AgentTool> {
    let t3_names = t3::names(env);
    let adapters: Vec<(&str, &str, &str, Box<dyn Fn() -> Found + Sync + Send>)> = vec![
        ("claude-code", "Claude Code", "claude", Box::new(|| claude::find(env, &t3_names))),
        ("codex", "Codex", "codex", Box::new(|| codex::find(env, &t3_names))),
        ("t3code", "T3 Code", "t3", Box::new(|| t3::find(env))),
        ("antigravity", "Antigravity CLI", "gemini", Box::new(|| antigravity::find(env))),
        ("gemini", "Gemini CLI", "gemini", Box::new(|| gemini::find(env))),
        ("cursor", "Cursor", "cursor", Box::new(|| cursor::find(env))),
        ("opencode", "opencode", "opencode", Box::new(|| opencode::find(env))),
    ];
    let mut tools: Vec<AgentTool> = adapters
        .par_iter()
        .filter_map(|(tool, name, icon, f)| {
            let found = f();
            if found.roots.iter().all(|r| !util::exists(r)) && found.sessions.is_empty() {
                return None;
            }
            Some(build(env, tool, name, icon, found, &t3_names))
        })
        .collect();
    tools.retain(|t| t.bytes > 0 || !t.projects.is_empty());
    tools.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    tools
}

fn build(env: &SessionEnv, tool: &str, name: &str, icon: &str, mut found: Found, names: &t3::Names) -> AgentTool {
    // One owner per path: drop paths another session already claimed, and guarded paths.
    let mut claimed: HashMap<PathBuf, ()> = HashMap::new();
    for r in found.sessions.iter_mut() {
        r.paths.retain(|p| util::exists(p) && !guard::never_delete(&env.home, p) && claimed.insert(p.clone(), ()).is_none());
    }
    found.sessions.retain(|r| !r.paths.is_empty());

    let measured: Vec<(Raw, Usage)> = found
        .sessions
        .into_par_iter()
        .map(|r| {
            let paths: Vec<String> = r.paths.iter().map(|p| s(p)).collect();
            let u = size::usage_all(&paths);
            (r, u)
        })
        .collect();

    let roots_total: u64 = found.roots.iter().filter(|r| util::exists(r)).map(|r| size::bytes(r)).sum();
    let outside: u64 = measured
        .iter()
        .flat_map(|(r, _)| r.paths.iter())
        .filter(|p| !found.roots.iter().any(|root| p.starts_with(root)))
        .map(|p| size::bytes(p))
        .sum();

    // Group by project.
    let mut projects: BTreeMap<String, AgentProject> = BTreeMap::new();
    let mut session_bytes = 0u64;
    for (r, u) in measured {
        session_bytes += u.bytes;
        let (key, pname, ppath) = project_of(env, &r, names);
        let last = max_opt(r.last_used, u.newest);
        let p = projects.entry(key).or_insert_with(|| AgentProject { name: pname, path: ppath, bytes: 0, last_used: None, sessions: vec![] });
        p.bytes += u.bytes;
        p.last_used = max_opt(p.last_used, last);
        p.sessions.push(AgentSession {
            id: r.id,
            title: r.title,
            bytes: u.bytes,
            started: r.started,
            last_used: last,
            paths: r.paths.iter().map(|p| s(p)).collect(),
            kind: r.kind.to_string(),
            busy: r.busy,
            warning: r.warning,
        });
    }
    let mut projects: Vec<AgentProject> = projects.into_values().collect();
    for p in projects.iter_mut() {
        p.sessions.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.last_used.cmp(&a.last_used)));
    }
    projects.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    // Shared pieces, then "everything else" so the parts add up to the whole.
    let mut shared: Vec<AgentShared> = found
        .shared
        .par_iter()
        .filter(|(_, p, _, _)| util::exists(p))
        .map(|(label, p, removable, why)| AgentShared {
            label: label.clone(),
            bytes: size::bytes(p),
            paths: if *removable && !guard::never_delete(&env.home, p) { vec![s(p)] } else { vec![] },
            why: why.clone(),
        })
        .collect();
    let shared_sum: u64 = shared.iter().map(|x| x.bytes).sum();
    let total = roots_total + outside;
    let rest = total.saturating_sub(session_bytes + shared_sum);
    if rest > 0 {
        shared.push(AgentShared {
            label: "Settings and other files".into(),
            bytes: rest,
            paths: vec![],
            why: format!("{}'s settings, sign-in, extensions and small files. Kept.", name),
        });
    }
    shared.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    AgentTool {
        tool: tool.into(),
        name: name.into(),
        icon: icon.into(),
        bytes: total,
        projects,
        roots: found.roots.iter().map(|p| s(p)).collect(),
        session_bytes,
        shared,
    }
}

/// (group key, display name, path) of a session's project.
fn project_of(env: &SessionEnv, r: &Raw, names: &t3::Names) -> (String, String, Option<String>) {
    let home = &env.home;
    let Some(p) = &r.project else {
        let n = r.project_name.clone().unwrap_or_else(|| "Unknown project".into());
        return (format!("name:{}", n), n, None);
    };
    let scratch = home.join(".t3/scratch");
    if p.starts_with(&scratch) {
        return ("scratch".into(), "Scratch chats".into(), Some(s(&scratch)));
    }
    let ps = s(p);
    if !p.starts_with(home) && ps.starts_with("/private/var/folders") || (!p.starts_with(home) && (ps.starts_with("/var/folders") || ps.starts_with("/tmp") || ps.starts_with("/private/tmp"))) {
        return ("temp".into(), "Temporary folders".into(), None);
    }
    if p == home {
        return (ps.clone(), "Home folder".into(), Some(ps));
    }
    // Machine-made workspaces (".../node-work/<hash>", ".../runs/<uuid>"): group by the folder
    // that holds them, or every job becomes its own project.
    let mut p = p.as_path();
    while p.parent().is_some() && looks_generated(&name_of(p)) {
        p = p.parent().unwrap();
    }
    let ps = s(p);
    if p == home.as_path() {
        return (ps.clone(), "Home folder".into(), Some(ps));
    }
    let name = r.project_name.clone().or_else(|| names.project_title(p)).unwrap_or_else(|| name_of(p));
    (ps.clone(), name, Some(ps))
}

/// A hash or uuid-like folder name.
fn looks_generated(name: &str) -> bool {
    let hexish = name.chars().filter(|c| c.is_ascii_hexdigit()).count();
    name.len() >= 12 && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_') && hexish * 10 >= name.len() * 8
}

/// Unix seconds from "2026-…" strings or numbers (s or ms).
pub(crate) fn time_of(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::String(t) => util::parse_time(t),
        serde_json::Value::Number(n) => n.as_i64().map(util::secs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_dedupes_paths_guards_config_and_adds_up() {
        let home = util::test_dir("build");
        let root = home.join(".tool");
        std::fs::create_dir_all(root.join("s")).unwrap();
        std::fs::write(root.join("s/a.jsonl"), vec![1u8; 40_000]).unwrap();
        std::fs::write(root.join("s/b.jsonl"), vec![1u8; 20_000]).unwrap();
        std::fs::write(root.join("config.toml"), b"x").unwrap();
        std::fs::write(root.join("big.db"), vec![1u8; 30_000]).unwrap();
        let env = SessionEnv::fixture(&home);
        let found = Found {
            roots: vec![root.clone()],
            sessions: vec![
                Raw { id: "a".into(), project: Some(home.join("proj")), paths: vec![root.join("s/a.jsonl"), root.join("config.toml")], kind: "session", ..Default::default() },
                Raw { id: "b".into(), project: Some(home.join("proj")), paths: vec![root.join("s/b.jsonl"), root.join("s/a.jsonl")], kind: "session", ..Default::default() },
                Raw { id: "gone".into(), paths: vec![root.join("nope")], kind: "session", ..Default::default() },
            ],
            shared: vec![("Database".into(), root.join("big.db"), false, "x".into())],
        };
        let t = build(&env, "x", "X", "x", found, &t3::Names::default());
        assert_eq!(t.projects.len(), 1);
        let p = &t.projects[0];
        assert_eq!(p.name, "proj");
        assert_eq!(p.sessions.len(), 2);
        let a = p.sessions.iter().find(|x| x.id == "a").unwrap();
        assert_eq!(a.paths, vec![s(&root.join("s/a.jsonl"))]); // config.toml guarded
        let b = p.sessions.iter().find(|x| x.id == "b").unwrap();
        assert_eq!(b.paths, vec![s(&root.join("s/b.jsonl"))]); // a.jsonl already a's
        assert_eq!(t.bytes, size::measure(&root).bytes);
        assert!(looks_generated("d9fa8a3fd7d5a1fa8e3cec62ffc0c5f9893c19ff") && looks_generated("ecc208bd-b829-40ea-a587-0123456789ab"));
        assert!(!looks_generated("TrendSway") && !looks_generated("deadbeef"));
        let parts = t.session_bytes + t.shared.iter().map(|x| x.bytes).sum::<u64>();
        assert_eq!(parts, t.bytes);
        assert!(t.shared.iter().any(|x| x.label == "Database" && x.paths.is_empty()));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
