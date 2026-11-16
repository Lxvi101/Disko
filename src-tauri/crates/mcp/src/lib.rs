//! `disko --mcp`: Disko's MCP server for coding agents (JSON-RPC 2.0 over stdio, one message per
//! line). Agents get Disko's whole-disk index and clutter findings in compact, size-sorted form,
//! can clean through Disko's safety rules (keep list wins, permanent delete only for rebuildable
//! data), and teach Disko: keep entries, new clutter sources (learned rules, optionally shared
//! anonymously), and routines.
//!
//! This crate knows nothing about the scan engine, clutter rules or apps: the app implements
//! [`Backend`] by wiring those crates together. Memory tools are implemented here directly on
//! `disko-memory`.

pub mod install;
mod tools;

#[cfg(test)]
mod tests;

use disko_core::{CleanResult, Finding, TreeNode, VolumeInfo};
use disko_memory::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

pub use install::{install, install_targets, is_installed, Installer, Target, TargetInfo};

/// How current the backend's picture of the disk is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Freshness {
    /// A scan is running; results are partial and keep improving.
    pub scanning: bool,
    /// "probes" | "walk" | "done" | "none" (no scan or snapshot yet).
    pub phase: String,
    /// 0..1 while scanning.
    pub fraction: f64,
    /// Seconds since the last complete scan (or snapshot), when known.
    pub age_s: Option<u64>,
    /// Files indexed so far.
    pub files: u64,
}

/// What to clean. The MCP layer has already removed kept, protected, busy and admin items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CleanRequest {
    /// Finding ids: each is removed with its own `action` (Delete only exists on `Safety::Safe`
    /// findings; Trash; or the owning tool's Command).
    pub ids: Vec<String>,
    /// Ids of findings to move to the Trash instead of their own action (agent asked for Trash).
    pub trash_ids: Vec<String>,
    /// Arbitrary absolute paths: always moved to the Trash (never deleted permanently).
    pub paths: Vec<String>,
}

/// Everything the MCP server needs from the rest of Disko. Implemented by the app.
///
/// Errors are short human sentences; they're shown to the agent as-is.
pub trait Backend: Send + Sync {
    /// Boot volume, including space a file walk can't see.
    fn volume(&self) -> Result<VolumeInfo, String>;
    /// Scan state. Must be cheap.
    fn freshness(&self) -> Freshness;
    /// Subtree at `path` (None = "/") down to `depth` levels, largest children first.
    fn tree(&self, path: Option<&str>, depth: u32) -> Result<TreeNode, String>;
    /// Largest folders/files below `under` (None = everything), flat, largest first. With
    /// `files_only`, only files. Folders should not be nested inside each other in the result.
    fn largest(&self, under: Option<&str>, limit: usize, files_only: bool) -> Result<Vec<TreeNode>, String>;
    /// Name search in the index (case-insensitive substring), flat.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<TreeNode>, String>;
    /// All current findings, including those produced by learned/community rules
    /// (`disko_memory::rules`). Unfiltered: the MCP layer applies the keep list.
    fn clutter(&self) -> Result<Vec<Finding>, String>;
    /// What Disko knows about one path: an object with any of `size`, `bytes`, `files`,
    /// `last_used` (unix s), `category`, `rule`, `finding_id`, `owner` (app/tool), `regrows`,
    /// `safety`, `why`, `busy`. The MCP layer adds keep status and learned rules.
    fn explain(&self, path: &str) -> Result<Value, String>;
    /// Clean. Must go through `disko_core::ops` and drop removed paths from the index.
    fn clean(&self, req: &CleanRequest) -> Result<CleanResult, String>;
    /// Installed apps (`AppInfo[]` as JSON).
    fn apps(&self) -> Result<Value, String>;
    /// Coding-agent tools with projects and sessions (`AgentTool[]` as JSON).
    fn sessions(&self) -> Result<Value, String>;
    /// Start a fresh scan (returns immediately).
    fn rescan(&self) -> Result<(), String>;
    /// Memory changed (keep list, rules, routines): re-evaluate findings, notify the UI.
    fn memory_changed(&self) {}
}

pub const INSTRUCTIONS: &str = "\
Disko indexes this Mac's whole disk and knows what's clutter. Use it instead of du/find: answers are instant and sizes are allocated bytes.

Start with `overview` (disk, scan state, clutter by category, routines). Then:
- Where did space go: `largest` (optionally `under` a folder), `tree` to drill in, `search` by name.
- What can go: `clutter` (filter by category/safety/query; sorted by size). `explain` any path: which rule, owner, regrows, keep status.
- Remove: `clean` with finding ids (or paths). Safe (rebuildable) findings are deleted, review findings go to the Trash (Undo: Finder › Put Back), tool-managed ones run the tool's own cleanup. Arbitrary paths only go to the Trash. Use `dry_run: true` to preview. Ask the user before removing review items or anything personal.
- The keep list always wins. When the user says to keep something, call `keep`. Never clean around it.
- Teach Disko: when you find a new kind of rebuildable clutter Disko didn't list (a tool cache, a build output folder), call `add_rule` with a ~/ glob (`**` for any depth, e.g. ~/**/.turbo). Set `share: true` only for generic, non-personal sources; it's queued anonymized for the community pack.
- Routines: `memory` lists them; `set_routine` (e.g. enable \"build-artifacts\": build output untouched for 2 days, every 2 days) and `run_routine`. Only enable or create routines the user asked for.
Sizes come as text and bytes. Paths are absolute. Results are capped; use `limit`.";

/// The MCP server: a backend plus the memory store.
pub struct Server {
    backend: Arc<dyn Backend>,
    store: Mutex<Store>,
}

impl Server {
    pub fn new(backend: Arc<dyn Backend>, store: Store) -> Server {
        Server { backend, store: Mutex::new(store) }
    }

    /// The real memory store in Disko's support folder.
    pub fn with_default_store(backend: Arc<dyn Backend>) -> Server {
        Server::new(backend, Store::open_default())
    }

    pub(crate) fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    pub(crate) fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Handle one JSON-RPC message (or batch). None for notifications.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        if let Value::Array(items) = msg {
            let out: Vec<Value> = items.iter().filter_map(|m| self.handle(m)).collect();
            return if out.is_empty() { None } else { Some(Value::Array(out)) };
        }
        let id = msg.get("id")?.clone(); // notifications get no reply
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = if ["2025-06-18", "2025-03-26", "2024-11-05"].contains(&asked) { asked } else { "2025-06-18" };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "disko", "title": "Disko", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = msg.pointer("/params/arguments").cloned().filter(Value::is_object).unwrap_or(json!({}));
                Ok(match tools::call(self, name, &args) {
                    Ok(v) => json!({ "content": [{ "type": "text", "text": v.to_string() }], "isError": false }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                })
            }
            m if m.starts_with("notifications/") => return None,
            _ => Err(json!({ "code": -32601, "message": format!("Method not found: {}", method) })),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        })
    }

    /// Handle one line of input; the reply line, if any.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => self.handle(&msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        reply.map(|r| r.to_string())
    }

    /// Serve until EOF.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if let Some(r) = self.handle_line(&line) {
                writeln!(output, "{}", r)?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Serve on stdin/stdout (what `disko --mcp` calls). Returns the process exit code.
    pub fn serve_stdio(&self) -> i32 {
        let stdin = std::io::stdin();
        match self.serve(stdin.lock(), std::io::stdout()) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }
}
