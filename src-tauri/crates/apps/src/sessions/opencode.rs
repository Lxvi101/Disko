//! opencode (~/.local/share/opencode/storage). `project/<pid>.json` names the worktree,
//! `session/<pid>/<sid>.json` has title and times, `message/<sid>/<mid>.json` the messages and
//! `part/<mid>/` their parts. A session is its JSON, its message folder and its messages' parts.

use super::{time_of, Found, Raw, SessionEnv};
use crate::util::{self, clean_title, name_of};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn find(env: &SessionEnv) -> Found {
    let root = env.home.join(".local/share/opencode");
    let storage = root.join("storage");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    let mut projects: HashMap<String, PathBuf> = HashMap::new();
    for p in util::children(&storage.join("project")) {
        let n = name_of(&p);
        let Some(id) = n.strip_suffix(".json") else { continue };
        if let Some(w) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| v["worktree"].as_str().map(PathBuf::from)) {
            projects.insert(id.to_string(), w);
        }
    }
    for pd in util::subdirs(&storage.join("session")) {
        let pid = name_of(&pd);
        for f in util::children(&pd) {
            let n = name_of(&f);
            let Some(sid) = n.strip_suffix(".json") else { continue };
            let v: serde_json::Value = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let mut paths = vec![f.clone()];
            let msgs = storage.join("message").join(sid);
            for m in util::children(&msgs) {
                if let Some(mid) = name_of(&m).strip_suffix(".json") {
                    let part = storage.join("part").join(mid);
                    if part.is_dir() {
                        paths.push(part);
                    }
                }
            }
            if msgs.is_dir() {
                paths.push(msgs);
            }
            let project = projects.get(&pid).cloned().or_else(|| v["directory"].as_str().map(PathBuf::from));
            found.sessions.push(Raw {
                id: sid.to_string(),
                title: v["title"].as_str().and_then(|t| clean_title(t, 120)),
                project_name: project.is_none().then(|| "Global".into()),
                project,
                started: time_of(&v["time"]["created"]),
                last_used: time_of(&v["time"]["updated"]),
                paths,
                kind: "session",
                ..Default::default()
            });
        }
    }
    found.shared = vec![("Logs".into(), root.join("log"), true, "Regrow.".into())];
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sessions_with_messages_and_parts() {
        let home = util::test_dir("opencode");
        let st = home.join(".local/share/opencode/storage");
        fs::create_dir_all(st.join("project")).unwrap();
        fs::create_dir_all(st.join("session/p1")).unwrap();
        fs::create_dir_all(st.join("message/ses_1")).unwrap();
        fs::create_dir_all(st.join("part/msg_1")).unwrap();
        fs::write(st.join("project/p1.json"), r#"{"worktree":"/Users/x/Proj"}"#).unwrap();
        fs::write(st.join("session/p1/ses_1.json"), r#"{"title":"Add a flag","time":{"created":1794824809000,"updated":1794830606000}}"#).unwrap();
        fs::write(st.join("message/ses_1/msg_1.json"), b"{}").unwrap();
        fs::write(st.join("part/msg_1/prt_1.json"), b"{}").unwrap();
        let f = find(&SessionEnv::fixture(&home));
        let s = &f.sessions[0];
        assert_eq!(s.title.as_deref(), Some("Add a flag"));
        assert_eq!(s.paths.len(), 3);
        assert_eq!(s.started, Some(1794824809));
        assert_eq!(s.project.as_deref(), Some(std::path::Path::new("/Users/x/Proj")));
        fs::remove_dir_all(&home).unwrap();
    }
}
