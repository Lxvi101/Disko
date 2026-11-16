//! Antigravity CLI (Google's agent CLI, ~/.gemini/antigravity-cli). A conversation is
//! `conversations/<id>.db` plus `brain/<id>/` and `annotations/<id>.pbtxt`; titles, times and
//! workspaces come from `conversation_summaries.db` (read-only).

use super::{Found, Raw, SessionEnv};
use crate::util::{self, clean_title, name_of};
use std::collections::HashMap;
use std::path::PathBuf;

struct Summary {
    title: Option<String>,
    modified: Option<i64>,
    workspace: Option<PathBuf>,
}

fn file_uri_path(u: &str) -> Option<PathBuf> {
    let p = u.strip_prefix("file://")?;
    // Minimal percent-decoding (spaces and the like).
    let mut out = Vec::new();
    let b = p.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).to_string()))
}

pub fn find(env: &SessionEnv) -> Found {
    let root = env.home.join(".gemini/antigravity-cli");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    if !root.is_dir() {
        return found;
    }
    let mut sums: HashMap<String, Summary> = HashMap::new();
    if let Some(c) = util::open_ro(&root.join("conversation_summaries.db")) {
        if let Ok(mut st) = c.prepare("SELECT conversation_id, title, preview, last_modified_time, workspace_uris FROM conversation_summaries") {
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?))
            });
            for (id, title, preview, modified, ws) in rows.into_iter().flatten().flatten() {
                let workspace = ws
                    .and_then(|w| serde_json::from_str::<Vec<String>>(&w).ok())
                    .and_then(|v| v.first().and_then(|u| file_uri_path(u)));
                let title = title.filter(|t| !t.trim().is_empty()).or(preview).and_then(|t| clean_title(&t, 120));
                sums.insert(id, Summary { title, modified: modified.and_then(|m| util::parse_time(&m)), workspace });
            }
        }
    }
    let mut by_id: HashMap<String, Raw> = HashMap::new();
    let mut add = |id: String, p: PathBuf| {
        let e = by_id.entry(id.clone()).or_insert_with(|| {
            let s = sums.get(&id);
            Raw {
                id: id.clone(),
                title: s.and_then(|s| s.title.clone()),
                project: s.and_then(|s| s.workspace.clone()),
                last_used: s.and_then(|s| s.modified),
                kind: "session",
                ..Default::default()
            }
        });
        e.paths.push(p);
    };
    for p in util::children(&root.join("conversations")) {
        if let Some(id) = name_of(&p).strip_suffix(".db") {
            add(id.to_string(), p.clone());
        }
    }
    for p in util::subdirs(&root.join("brain")) {
        add(name_of(&p), p);
    }
    for p in util::children(&root.join("annotations")) {
        if let Some(id) = name_of(&p).strip_suffix(".pbtxt") {
            add(id.to_string(), p.clone());
        }
    }
    // Notes whose conversation is gone: one leftovers entry instead of thousands of tiny rows.
    let mut leftovers = Vec::new();
    by_id.retain(|_, r| {
        let only_notes = r.paths.iter().all(|p| name_of(p).ends_with(".pbtxt"));
        if only_notes {
            leftovers.append(&mut r.paths);
        }
        !only_notes
    });
    if !leftovers.is_empty() {
        by_id.insert(
            "antigravity:leftovers".into(),
            Raw { id: "antigravity:leftovers".into(), title: Some("Notes of deleted conversations".into()), project_name: Some("Left behind".into()), paths: leftovers, kind: "session", ..Default::default() },
        );
    }
    for r in by_id.values_mut() {
        if r.project.is_none() && r.project_name.is_none() {
            r.project_name = Some("Unknown workspace".into());
        }
        if r.last_used.is_some_and(|t| env.now - t < 120) {
            r.busy = Some("Active a moment ago".into());
        }
    }
    found.sessions = by_id.into_values().collect();
    found.shared = vec![
        ("Conversation index".into(), root.join("conversation_summaries.db"), false, "Titles and search index of all conversations.".into()),
        ("Logs".into(), root.join("log"), true, "Regrow.".into()),
        ("Cache".into(), root.join("cache"), true, "Regrows.".into()),
        ("Program".into(), root.join("bin"), false, "The CLI itself.".into()),
    ];
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn conversations_brain_and_summaries() {
        let home = util::test_dir("agy");
        let root = home.join(".gemini/antigravity-cli");
        let id = "6008cd6a-98b2-4b69-9316-d0fb9a15a1ea";
        fs::create_dir_all(root.join("conversations")).unwrap();
        fs::create_dir_all(root.join("brain").join(id).join("scratch")).unwrap();
        fs::create_dir_all(root.join("annotations")).unwrap();
        fs::write(root.join("conversations").join(format!("{id}.db")), vec![1u8; 8000]).unwrap();
        fs::write(root.join("annotations").join(format!("{id}.pbtxt")), b"x").unwrap();
        let c = rusqlite::Connection::open(root.join("conversation_summaries.db")).unwrap();
        c.execute_batch("CREATE TABLE conversation_summaries(conversation_id TEXT, title TEXT, preview TEXT, last_modified_time TEXT, workspace_uris TEXT);").unwrap();
        c.execute("INSERT INTO conversation_summaries VALUES(?1,'','Render the video','2026-11-16 12:11:46.213632+00:00','[\"file:///Users/x/My%20Proj\"]')", [id]).unwrap();
        drop(c);
        let f = find(&SessionEnv::fixture(&home));
        assert_eq!(f.sessions.len(), 1);
        let s = &f.sessions[0];
        assert!(!s.paths.is_empty());
        assert_eq!(s.paths.len(), 3);
        assert_eq!(s.title.as_deref(), Some("Render the video"));
        assert_eq!(s.project.as_deref(), Some(std::path::Path::new("/Users/x/My Proj")));
        assert_eq!(s.last_used, Some(1794831106));
        fs::remove_dir_all(&home).unwrap();
    }
}
