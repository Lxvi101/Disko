//! The shapes this crate hands to the app, the UI and agents. snake_case JSON, mirrored in
//! `src/lib/types.ts`. Fields marked "added" go beyond the minimum in docs/contract.md.

use disko_core::CleanResult;
use serde::{Deserialize, Serialize};

/// One installed app with everything that belongs to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    /// The .app bundle.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// "app:<path>" so the UI uses the app's own icon.
    pub icon: String,
    /// Allocated bytes of the bundle (plus its vendor folder when the app lives alone in one,
    /// e.g. "/Applications/Adobe Photoshop 2026/").
    pub app_bytes: u64,
    /// Allocated bytes of `data_paths`.
    pub data_bytes: u64,
    /// Everything outside the bundle that belongs only to this app (removed with it).
    pub data_paths: Vec<String>,
    /// Best-effort unix seconds the app was last used (see `apps::last_used`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
    pub running: bool,
    /// Ships with macOS (signed by Apple's "Software Signing"): never uninstallable.
    pub apple: bool,
    /// added: what `uninstall` actually removes when it isn't just `path` (the vendor folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_root: Option<String>,
    /// added: data this app shares with other installed apps (same team group container,
    /// vendor folder). Shown, never removed by an uninstall.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_paths: Vec<String>,
    /// added: the vendor's own uninstaller; `uninstall` refuses and points here instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uninstaller: Option<String>,
    /// added: data/bundle paths owned by root (need one admin prompt to remove).
    #[serde(default)]
    pub admin: bool,
    /// added: "app-store" | "developer-id" | "apple" | "unsigned" | "other".
    pub source: String,
}

/// One coding agent (Claude Code, Codex, T3 Code …) and where its disk space went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTool {
    /// "claude-code" | "codex" | "t3code" | "antigravity" | "gemini" | "cursor" | "opencode"
    pub tool: String,
    pub name: String,
    /// Glyph key for the UI ("claude", "codex", "t3", "gemini", "cursor", "opencode").
    pub icon: String,
    /// Everything the tool keeps in its folders (du of `roots`).
    pub bytes: u64,
    pub projects: Vec<AgentProject>,
    /// added: the tool's folders that `bytes` measures.
    pub roots: Vec<String>,
    /// added: sum of all sessions' bytes (what deleting sessions can give back).
    pub session_bytes: u64,
    /// added: big things that belong to the tool as a whole (its database, logs, packages,
    /// settings). Shown so the numbers add up. `paths` empty means "manage it in the tool".
    pub shared: Vec<AgentShared>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProject {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
    pub sessions: Vec<AgentSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
    /// What deleting this session moves to the Trash. Never credentials or settings.
    pub paths: Vec<String>,
    /// added: "session" | "worktree" | "scratch".
    pub kind: String,
    /// added: the session is in use right now ("Running now"); deleting skips it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy: Option<String>,
    /// added: worth knowing before deleting ("3 uncommitted files", "2 commits not pushed").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentShared {
    pub label: String,
    pub bytes: u64,
    /// Removable paths (rebuildable caches, logs). Empty for things the tool must manage itself.
    pub paths: Vec<String>,
    pub why: String,
}

/// `uninstall` result: a `CleanResult` plus what an admin prompt deleted permanently and notes.
/// Serialises flat, so it reads as a `CleanResult` with two extra fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Uninstall {
    #[serde(flatten)]
    pub result: CleanResult,
    /// Root-owned paths deleted permanently through one administrator prompt (no Undo).
    #[serde(default)]
    pub deleted: Vec<String>,
    /// Plain-language notes ("Adobe apps are removed with Adobe's uninstaller: …").
    #[serde(default)]
    pub notes: Vec<String>,
}
