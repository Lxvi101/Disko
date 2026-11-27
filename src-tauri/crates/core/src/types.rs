use serde::{Deserialize, Serialize};

/// What kind of clutter something is. The UI gives each one an icon and a colour; keep the list
/// short. Order here is the order the UI shows categories in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Build output: Rust target, node_modules, .build, DerivedData, __pycache__, .next …
    Build,
    /// App and system caches that regrow on their own.
    Cache,
    /// Package manager stores and downloads (npm, pnpm, cargo registry, Homebrew, pip, uv …).
    Packages,
    /// Xcode / simulators / device support / archives.
    Developer,
    /// Leftovers of apps that are no longer installed, dead launch agents, orphaned containers.
    Leftovers,
    /// Older versions superseded by a newer installed one (toolchains, CLIs, app versions, extensions).
    OldVersions,
    /// Installers and disk images that already did their job (.dmg, .pkg, .xip, IPSW, zips of extracted files).
    Installers,
    /// Logs, crash and diagnostic reports.
    Logs,
    /// Coding agent histories (Claude Code, Codex, T3 Code, Cursor …) and their worktrees.
    AgentData,
    /// Docker / VM disk images and their caches.
    Containers,
    /// Optional downloadable content: AI models, offline docs, voices, extra simulator runtimes.
    Extras,
    /// Big personal files nobody opened in a long time (only ever suggested, never automatic).
    Forgotten,
    /// The Trash and Disko's own leftovers.
    Trash,
}

impl Category {
    pub const ALL: [Category; 13] = [
        Category::Build,
        Category::Cache,
        Category::Packages,
        Category::Developer,
        Category::Leftovers,
        Category::OldVersions,
        Category::Installers,
        Category::Logs,
        Category::AgentData,
        Category::Containers,
        Category::Extras,
        Category::Forgotten,
        Category::Trash,
    ];
}

/// How sure Disko is that removing this is harmless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Safety {
    /// Rebuildable or regrowing by design. Fine to clean in one tap or on a routine.
    Safe,
    /// Probably clutter, but it's the user's call (a forgotten project, an old app, a big video).
    Review,
}

/// How Disko gets rid of something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Action {
    /// Delete right away. Only for rebuildable/regrowing data (Safety::Safe).
    Delete,
    /// Move to the macOS Trash so the user can put it back.
    Trash,
    /// Run the owning tool's own cleanup command instead of touching its files
    /// (e.g. `["brew","cleanup","--prune=all"]`, `["xcrun","simctl","delete","unavailable"]`).
    Command { argv: Vec<String>, label: String },
}

/// Who taught Disko about this kind of clutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Shipped with Disko.
    #[default]
    Builtin,
    /// The user (e.g. "always clean this").
    User,
    /// An agent, through the MCP server.
    Agent,
    /// The anonymous community rule pack.
    Community,
}

/// One thing Disko suggests removing. May cover several paths (e.g. "3 old Node versions").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable across scans for the same rule + paths, so selections and "keep" survive rescans.
    /// Build it with [`finding_id`].
    pub id: String,
    /// Rule that produced it, e.g. "build.node-modules", "leftovers.app-support".
    pub rule: String,
    pub category: Category,
    /// Big, human title. A project, app or tool name, not a path: "Disko", "Adobe After Effects 2026".
    pub title: String,
    /// Optional short second line: "node_modules", "3 versions", "Simulator runtime".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Icon key for the UI. Either a known glyph ("node", "rust", "xcode", "python", "docker",
    /// "brew", "claude", "codex", "t3", "trash", "log", "box", "app" …) or "app:<absolute .app path>"
    /// to use that app's own icon.
    pub icon: String,
    /// Absolute paths this finding removes. Never overlapping with each other.
    pub paths: Vec<String>,
    /// Allocated bytes on disk (what you'd get back, before APFS clone/snapshot caveats).
    pub bytes: u64,
    /// Newest modification time inside, unix seconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
    pub safety: Safety,
    pub action: Action,
    /// Comes back by itself after cleaning (caches, build output). Shown as a small loop icon.
    pub regrows: bool,
    /// One short sentence of evidence: "Xcode 16.2 is installed; this is 15.4's", "No app owns com.foo".
    pub why: String,
    /// Set when something is using it right now ("Xcode is running"). The UI greys it out and
    /// cleaning skips it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy: Option<String>,
    /// Needs an administrator password to remove.
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub source: Source,
}

/// Stable id for a finding: rule + sorted paths, hashed (FNV-1a, hex).
pub fn finding_id(rule: &str, paths: &[String]) -> String {
    let mut sorted: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    sorted.sort_unstable();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in rule.bytes().chain(sorted.iter().flat_map(|p| std::iter::once(0u8).chain(p.bytes()))) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{}:{:016x}", rule, h)
}

/// A node of the disk tree as handed to the UI / agents (a view, not the engine's storage).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    /// "dir" | "file" | "other" (the engine's aggregate of small files or of hidden children).
    pub kind: String,
    /// Allocated bytes, including everything below.
    pub bytes: u64,
    /// Number of files below (1 for a file).
    pub files: u64,
    /// Newest mtime below, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<i64>,
    /// Category when this node is (or is inside) a known kind of clutter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clutter: Option<Category>,
    /// Why this folder couldn't be read fully ("Needs Full Disk Access", "Permission denied").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Present when requested depth reaches this node; largest first. Small children are folded
    /// into one `kind: "other"` child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    /// The protected-path guard refuses to remove it (system folders, top-level personal folders,
    /// credentials, repositories' history …), so the UI doesn't offer to.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub protected: bool,
}

/// Where the space on the boot volume went, including what a file walk can't see.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VolumeInfo {
    pub name: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    /// Space macOS says it can free on its own (purgeable), when known.
    #[serde(default)]
    pub purgeable: u64,
    /// Bytes the scan found in files.
    #[serde(default)]
    pub scanned: u64,
    /// Space used but not seen as files, explained piece by piece (local snapshots, other APFS
    /// volumes in the container like Preboot/VM/Recovery, folders we couldn't read, the rest).
    #[serde(default)]
    pub hidden: Vec<HiddenSpace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiddenSpace {
    /// "snapshots" | "system" | "vm" | "unreadable" | "purgeable" | "other"
    pub kind: String,
    pub label: String,
    pub bytes: u64,
    pub why: String,
}

/// Scan progress, pushed a few times per second while scanning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ScanProgress {
    /// "probes" (quick targeted checks) | "walk" (full disk) | "done"
    pub phase: String,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
    /// Folder being read right now (for a subtle ticker).
    #[serde(default)]
    pub current: String,
    /// 0..1 estimate (bytes seen / volume used).
    pub fraction: f64,
}

/// Result of a clean. The engine has already dropped removed paths from its tree when this returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CleanResult {
    pub freed: u64,
    /// Finding ids fully removed.
    pub removed: Vec<String>,
    /// Paths moved to the Trash, with where they ended up, so Undo can put them back.
    pub trashed: Vec<Trashed>,
    pub failed: Vec<CleanFailure>,
    /// Skipped because they're in use or on the keep list.
    pub skipped: Vec<CleanFailure>,
    /// Paths removed permanently (rebuildable data, or root-owned items removed with a password).
    #[serde(default)]
    pub deleted: Vec<String>,
    /// Short notes for the user ("Adobe apps uninstall with Adobe's own uninstaller").
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trashed {
    pub from: String,
    pub to: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanFailure {
    pub path: String,
    pub error: String,
}

/// Hook the clutter rules give the engine, so name-based clutter (node_modules, target, caches …)
/// is recognised during the single disk walk instead of in a second pass.
///
/// Matches don't nest: once a directory matched, nothing below it is offered to the matcher.
pub trait WalkMatcher: Send + Sync {
    /// Called for every directory the walk enters. `siblings` are the names next to it (the
    /// listing of its parent), so rules like "target next to Cargo.toml" need no extra I/O.
    /// Return a tag to have the engine report this directory's totals once its subtree is done.
    fn on_dir(&self, path: &std::path::Path, name: &str, siblings: &[&str]) -> Option<u32>;
    /// Called for every file of at least 1 MiB that isn't inside a matched directory.
    fn on_file(&self, path: &std::path::Path, name: &str, bytes: u64, mtime: i64) -> Option<u32>;
    /// Category of a tag, used to colour the map.
    fn category(&self, tag: u32) -> Category;
}

/// A directory (or file) the matcher tagged, with its totals once known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Matched {
    pub tag: u32,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub newest: Option<i64>,
}
