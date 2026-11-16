//! Cursor. The agent CLI keeps chats in `~/.cursor/chats/<workspace hash>/<chat id>/` and
//! transcripts in `~/.cursor/projects/<slug>/agent-transcripts/`; the editor keeps one folder per
//! opened workspace in `~/Library/Application Support/Cursor/User/workspaceStorage/<hash>/`
//! (its `workspace.json` names the folder). Each of those is a session; their project is the
//! workspace folder when known.

use super::{claude::decode_slug, Found, Raw, SessionEnv};
use crate::util::{self, name_of};
use std::path::PathBuf;

fn workspace_folder(dir: &std::path::Path) -> Option<PathBuf> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("workspace.json")).ok()?).ok()?;
    let uri = v["folder"].as_str().or_else(|| v["workspace"].as_str())?;
    Some(PathBuf::from(uri.strip_prefix("file://")?.replace("%20", " ")))
}

pub fn find(env: &SessionEnv) -> Found {
    let dot = env.home.join(".cursor");
    let app = env.home.join("Library/Application Support/Cursor");
    let mut found = Found { roots: vec![dot.clone(), app.clone()], ..Default::default() };
    for ws in util::subdirs(&dot.join("chats")) {
        for chat in util::subdirs(&ws) {
            found.sessions.push(Raw {
                id: name_of(&chat),
                project_name: Some(format!("Workspace {}", name_of(&ws).chars().take(8).collect::<String>())),
                last_used: crate::size::own_mtime(&chat),
                paths: vec![chat],
                kind: "session",
                ..Default::default()
            });
        }
    }
    for pd in util::subdirs(&dot.join("projects")) {
        let project = Some(decode_slug(&name_of(&pd)));
        for t in util::children(&pd.join("agent-transcripts")) {
            found.sessions.push(Raw {
                id: name_of(&t),
                project: project.clone(),
                last_used: crate::size::own_mtime(&t),
                paths: vec![t],
                kind: "session",
                ..Default::default()
            });
        }
    }
    for ws in util::subdirs(&app.join("User/workspaceStorage")) {
        let folder = workspace_folder(&ws);
        found.sessions.push(Raw {
            id: name_of(&ws),
            title: Some("Editor workspace state".into()),
            project_name: folder.is_none().then(|| "Unknown workspace".into()),
            project: folder,
            last_used: crate::size::own_mtime(&ws),
            paths: vec![ws],
            kind: "session",
            ..Default::default()
        });
    }
    found.shared = vec![
        ("Editor caches".into(), app.join("Cache"), true, "Regrow.".into()),
        ("Editor code cache".into(), app.join("CachedData"), true, "Regrows.".into()),
        ("Extensions".into(), dot.join("extensions"), false, "Installed editor extensions.".into()),
    ];
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn chats_transcripts_and_workspace_storage() {
        let home = util::test_dir("cursor");
        fs::create_dir_all(home.join(".cursor/chats/ab12cd34ef/chat-1")).unwrap();
        fs::write(home.join(".cursor/chats/ab12cd34ef/chat-1/store.db"), b"x").unwrap();
        let ws = home.join("Library/Application Support/Cursor/User/workspaceStorage/h1");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("workspace.json"), r#"{"folder":"file:///Users/x/My%20App"}"#).unwrap();
        fs::write(home.join(".cursor/cli-config.json"), b"{}").unwrap();
        let f = find(&SessionEnv::fixture(&home));
        assert_eq!(f.sessions.len(), 2);
        assert!(f.sessions.iter().any(|s| s.project.as_deref() == Some(std::path::Path::new("/Users/x/My App"))));
        fs::remove_dir_all(&home).unwrap();
    }
}
