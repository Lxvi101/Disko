//! Disko's memory: what the user wants to keep, what kinds of clutter Disko learned about (from the
//! user, from agents over MCP, and from the anonymous community pack), and routines like "delete
//! build artifacts every other day".
//!
//! Everything lives in one JSON file, `memory.json`, in Disko's support folder. Both the app and
//! the MCP server process (`disko --mcp`) read and write it; writes are atomic and serialized with a
//! lock file, and readers reload when the file's mtime changes ([`Store::refresh`]).
//!
//! Nothing here deletes files. Routines only *select* findings; the caller cleans them through
//! `disko_core::ops`.

pub mod community;
pub mod glob;
pub mod keep;
pub mod rules;
pub mod routines;
pub mod share;
pub mod store;

pub use glob::Glob;
pub use keep::KeepSet;
pub use rules::{validate_rule, LearnedMatcher};
pub use routines::select;
pub use share::Submission;
pub use store::Store;

use disko_core::{Category, Safety, Source};
use serde::{Deserialize, Serialize};

/// Version of `memory.json`. Bump when a change can't be read by older Disko versions.
pub const SCHEMA_VERSION: u32 = 1;

/// Everything Disko remembers. Sent to the UI as-is (`memory` command / event).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Memory {
    #[serde(default)]
    pub keep: Vec<KeepEntry>,
    #[serde(default)]
    pub rules: Vec<LearnedRule>,
    #[serde(default)]
    pub routines: Vec<Routine>,
    #[serde(default)]
    pub community: CommunityStatus,
}

/// Something the user (or an agent on their behalf) wants Disko to leave alone, forever.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeepEntry {
    pub id: String,
    /// Absolute path, or a glob starting with `~/` or `/` (`*`, `?`, `**`). Anything inside a match
    /// is kept too, and so is any folder that contains a match.
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default = "user_source")]
    pub source: Source,
    #[serde(default)]
    pub added: i64,
}

/// A new source of clutter Disko learned about. Becomes findings with rule id `learned.<id>`
/// (or `community.<id>` for the community pack).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedRule {
    #[serde(default)]
    pub id: String,
    /// `~/`-relative or absolute glob of what to remove (`*`, `?` within a name, `**` for any depth).
    pub pattern: String,
    pub category: Category,
    pub title: String,
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub regrows: bool,
    pub safety: Safety,
    #[serde(default = "user_source")]
    pub source: Source,
    #[serde(default)]
    pub added: i64,
    /// Share this rule (anonymized) with other Disko users.
    #[serde(default)]
    pub shared: bool,
}

/// "Clean X every N days". Disabled routines are suggestions the user can switch on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub select: RoutineSelect,
    #[serde(default = "two")]
    pub every_days: u32,
    /// Only take findings untouched for at least this many days (by their newest mtime).
    #[serde(default)]
    pub min_age_days: u32,
    #[serde(default)]
    pub enabled: bool,
    /// Also take `Safety::Review` findings (moved to the Trash). Off unless the user explicitly
    /// made a routine for review items.
    #[serde(default, skip_serializing_if = "is_false")]
    pub review: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_freed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutineSelect {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<Category>,
    /// Finding rule ids (`build.node-modules`), rule-id prefixes (`build`), or learned/community
    /// rule ids (`gradle-caches` matches `learned.gradle-caches` and `community.gradle-caches`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityStatus {
    /// Use the community rule pack (fetched from GitHub at most once a day).
    pub enabled: bool,
    /// Number of usable rules in the current pack.
    #[serde(default)]
    pub rules: usize,
    /// When the pack in use was fetched (unix seconds); none = the copy bundled with Disko.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<i64>,
    /// Last fetch attempt (unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<i64>,
    /// Where anonymized shared rules are POSTed. `DISKO_COMMUNITY_URL` overrides it. None = they
    /// stay queued in the outbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_url: Option<String>,
    /// Shared rules waiting in the outbox.
    #[serde(default)]
    pub queued: usize,
}

impl Default for CommunityStatus {
    fn default() -> Self {
        CommunityStatus { enabled: true, rules: 0, updated: None, checked: None, submit_url: None, queued: 0 }
    }
}

/// One change to the memory. Sent by the UI (`memory_update`) and by agents over MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum MemoryOp {
    Keep {
        pattern: String,
        #[serde(default)]
        note: Option<String>,
        /// Defaults to `user`; the MCP server sets `agent`.
        #[serde(default)]
        source: Option<Source>,
    },
    /// By entry id, or by its exact pattern.
    Unkeep {
        id: String,
        /// `agent` (set by the MCP server): only entries an agent added may be removed.
        #[serde(default)]
        source: Option<Source>,
    },
    AddRule { rule: LearnedRule },
    RemoveRule { id: String },
    /// Turn sharing of a learned rule on or off.
    ShareRule { id: String, shared: bool },
    /// Insert or replace (by id) a routine.
    SetRoutine { routine: Routine },
    RemoveRoutine { id: String },
    Community { enabled: bool },
    /// Where to send shared rules (None/empty = keep them queued).
    CommunityUrl {
        #[serde(default)]
        url: Option<String>,
    },
}

fn user_source() -> Source {
    Source::User
}
fn two() -> u32 {
    2
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Short stable hash (FNV-1a, 8 hex chars) for ids.
pub(crate) fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h >> 32) as u32 ^ h as u32)
}

/// "Gradle caches!" → "gradle-caches".
pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Decimal sizes like Finder: "1.2 GB".
pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", b)
    } else if v < 10.0 {
        format!("{:.1} {}", v, U[i])
    } else {
        format!("{:.0} {}", v, U[i])
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fresh temp dir per call; removed by the caller (or left in $TMPDIR).
    pub fn tempdir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "disko-memory-test-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_json_shapes() {
        let op: MemoryOp = serde_json::from_str(r#"{"type":"keep","pattern":"~/x"}"#).unwrap();
        assert!(matches!(op, MemoryOp::Keep { .. }));
        let op: MemoryOp = serde_json::from_str(
            r#"{"type":"add-rule","rule":{"pattern":"~/.gradle/caches","category":"packages","title":"Gradle","safety":"safe"}}"#,
        )
        .unwrap();
        match op {
            MemoryOp::AddRule { rule } => {
                assert_eq!(rule.source, Source::User);
                assert!(!rule.shared);
            }
            _ => panic!(),
        }
        let r: Routine = serde_json::from_str(r#"{"title":"x","select":{"categories":["build"]}}"#).unwrap();
        assert_eq!(r.every_days, 2);
    }

    #[test]
    fn helpers() {
        assert_eq!(slug("Gradle caches!"), "gradle-caches");
        assert_eq!(fmt_bytes(999), "999 B");
        assert_eq!(fmt_bytes(1_234_000_000), "1.2 GB");
        assert_eq!(fmt_bytes(45_000_000), "45 MB");
    }
}
