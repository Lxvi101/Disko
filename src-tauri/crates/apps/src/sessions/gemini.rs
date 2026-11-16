//! Gemini CLI (~/.gemini/tmp/<project hash>/): `chats/session-*.json` per session, plus the
//! project's checkpoints and logs. The project folder is read from `.project_root` when the CLI
//! wrote one; otherwise the hash is all we have.

use super::{time_of, Found, Raw, SessionEnv};
use crate::util::{self, clean_title, name_of};
use std::path::PathBuf;

pub fn find(env: &SessionEnv) -> Found {
    let root = env.home.join(".gemini/tmp");
    let mut found = Found { roots: vec![root.clone()], ..Default::default() };
    for pd in util::subdirs(&root) {
        let project = std::fs::read_to_string(pd.join(".project_root")).ok().map(|s| PathBuf::from(s.trim())).filter(|p| p.is_absolute());
        let short = name_of(&pd).chars().take(8).collect::<String>();
        for f in util::children(&pd.join("chats")) {
            let n = name_of(&f);
            if !(n.starts_with("session-") && n.ends_with(".json")) {
                continue;
            }
            let v: serde_json::Value = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let first_user = v["messages"].as_array().and_then(|m| m.iter().find(|x| x["type"] == "user")).and_then(|x| x["content"].as_str().map(str::to_string).or_else(|| x["content"][0]["text"].as_str().map(str::to_string)));
            found.sessions.push(Raw {
                id: v["sessionId"].as_str().map(str::to_string).unwrap_or_else(|| n.trim_end_matches(".json").to_string()),
                title: first_user.and_then(|t| clean_title(&t, 120)),
                project: project.clone(),
                project_name: project.is_none().then(|| format!("Project {}", short)),
                started: time_of(&v["startTime"]),
                last_used: time_of(&v["lastUpdated"]),
                paths: vec![f],
                kind: "session",
                ..Default::default()
            });
        }
        // Checkpoints and shell history of the project go with its most recent session's
        // project row as one "project files" entry.
        let rest: Vec<PathBuf> = util::children(&pd).into_iter().filter(|p| name_of(p) != "chats" && name_of(p) != ".project_root").collect();
        if !rest.is_empty() {
            found.sessions.push(Raw {
                id: format!("gemini:{}", name_of(&pd)),
                title: Some("Checkpoints and logs".into()),
                project: project.clone(),
                project_name: project.is_none().then(|| format!("Project {}", short)),
                paths: rest,
                kind: "session",
                ..Default::default()
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn chats_per_project_hash() {
        let home = util::test_dir("gemini");
        let pd = home.join(".gemini/tmp/abcdef0123456789");
        fs::create_dir_all(pd.join("chats")).unwrap();
        fs::write(pd.join(".project_root"), "/Users/x/Proj\n").unwrap();
        fs::write(pd.join("logs.json"), b"[]").unwrap();
        let chat = serde_json::json!({"sessionId":"s1","startTime":"2026-11-01T10:00:00Z","lastUpdated":"2026-11-01T11:00:00Z","messages":[{"type":"user","content":"Write tests"}]});
        fs::write(pd.join("chats/session-2026-11-01T10-00-s1.json"), chat.to_string()).unwrap();
        let f = find(&SessionEnv::fixture(&home));
        let s = f.sessions.iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(s.title.as_deref(), Some("Write tests"));
        assert_eq!(s.project.as_deref(), Some(std::path::Path::new("/Users/x/Proj")));
        assert!(f.sessions.iter().any(|s| s.title.as_deref() == Some("Checkpoints and logs")));
        fs::remove_dir_all(&home).unwrap();
    }
}
