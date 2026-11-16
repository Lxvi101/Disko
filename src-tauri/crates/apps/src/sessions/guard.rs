//! What a session delete must never touch, whatever an adapter (or an agent) says.

use std::path::Path;

/// File names that hold credentials, settings or instructions.
const NAMES: &[&str] = &[
    "auth.json",
    ".credentials.json",
    "credentials.json",
    "config.toml",
    "config.json",
    "settings.json",
    "settings.local.json",
    "cli-config.json",
    "mcp.json",
    "mcp_config.json",
    ".mcp.json",
    "hooks.json",
    "keybindings.json",
    "CLAUDE.md",
    "AGENTS.md",
    "GEMINI.md",
    "MEMORY.md",
];

/// Folder names that hold settings, extensions or memory (anything inside is off limits).
const DIRS: &[&str] = &["memory", "memories", "skills", "plugins", "agents", "secrets", "clerk", "commands", "rules", "mcp", ".git"];

/// Tool roots themselves are never a session path.
const ROOTS: &[&str] = &[".claude", ".codex", ".t3", ".gemini", ".cursor", ".config/opencode", ".local/share/opencode"];

pub fn never_delete(home: &Path, p: &Path) -> bool {
    if disko_core::paths::is_protected(p) {
        return true;
    }
    if ROOTS.iter().any(|r| p == home.join(r)) {
        return true;
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if NAMES.contains(&name.as_str()) || name.starts_with("settings.json") || name.starts_with("config.toml") || name.starts_with("auth.json") {
        return true;
    }
    // Inside a tool root, a guarded folder anywhere on the way is off limits. (Worktrees outside
    // tool roots are whole checkouts and may legitimately contain e.g. a `skills/` folder.)
    let rel = ROOTS.iter().find_map(|r| p.strip_prefix(home.join(r)).ok());
    if let Some(rel) = rel {
        if rel.components().any(|c| DIRS.contains(&c.as_os_str().to_string_lossy().as_ref())) {
            // ...except inside worktrees / scratch, which are whole checkouts.
            let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
            if first != "worktrees" && first != "scratch" {
                return true;
            }
        }
        // T3's / Codex's worktree and scratch containers themselves.
        if rel == Path::new("worktrees") || rel == Path::new("scratch") || rel == Path::new("projects") || rel == Path::new("sessions") || rel == Path::new("userdata") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_config_memory_and_roots() {
        let h = Path::new("/Users/x");
        for bad in [
            "/Users/x/.codex/auth.json",
            "/Users/x/.codex/config.toml.pre-hub-1",
            "/Users/x/.claude/.credentials.json",
            "/Users/x/.claude/settings.json",
            "/Users/x/.claude/projects/-Users-x-a/memory",
            "/Users/x/.claude/projects/-Users-x-a/memory/MEMORY.md",
            "/Users/x/.claude/skills/foo",
            "/Users/x/.claude/plugins",
            "/Users/x/.t3/userdata/secrets",
            "/Users/x/.claude",
            "/Users/x/.t3/worktrees",
            "/Users/x/.claude/projects",
            "/",
        ] {
            assert!(never_delete(h, Path::new(bad)), "{bad}");
        }
        for ok in [
            "/Users/x/.claude/projects/-Users-x-a/abc.jsonl",
            "/Users/x/.claude/projects/-Users-x-a/abc",
            "/Users/x/.codex/sessions/2026/11/03/rollout-x.jsonl",
            "/Users/x/.t3/worktrees/Repo/t3-1234",
            "/Users/x/.t3/scratch/2026-11-01-thing-abcd1234",
        ] {
            assert!(!never_delete(h, Path::new(ok)), "{ok}");
        }
    }
}
