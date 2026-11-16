//! `disko_mcp::Backend` on top of the Hub, so agents see exactly what the app sees.

use crate::hub::Hub;
use disko_core::{CleanResult, Finding, TreeNode, VolumeInfo};
use disko_mcp::{Backend, CleanRequest, Freshness};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct HubBackend(pub Arc<Hub>);

fn no_index() -> String {
    "Disko hasn't indexed the disk yet; a scan is starting. Try again in a minute.".into()
}

impl Backend for HubBackend {
    fn volume(&self) -> Result<VolumeInfo, String> {
        Ok(self.0.volume())
    }

    fn freshness(&self) -> Freshness {
        let p = self.0.progress();
        let scanning = self.0.is_scanning();
        let has_tree = self.0.engine.tree(None, 0).is_some();
        Freshness {
            scanning,
            phase: if scanning { p.phase.clone() } else if has_tree { "done".into() } else { "none".into() },
            fraction: if scanning { p.fraction } else if has_tree { 1.0 } else { 0.0 },
            age_s: self.0.snapshot_age().map(|d| d.as_secs()),
            files: p.files,
        }
    }

    fn tree(&self, path: Option<&str>, depth: u32) -> Result<TreeNode, String> {
        self.0.engine.tree(path, depth).ok_or_else(|| match path {
            Some(p) if self.0.engine.tree(None, 0).is_some() => format!("{p} isn't in the index (missing, excluded or unreadable)."),
            _ => no_index(),
        })
    }

    fn largest(&self, under: Option<&str>, limit: usize, files_only: bool) -> Result<Vec<TreeNode>, String> {
        if files_only {
            return Ok(self.0.engine.largest_files(under, limit));
        }
        // Largest folders without nesting: greedily expand the biggest folder until `limit`
        // leaves, like peeling the map.
        let root = self.0.engine.tree(under, 1).ok_or_else(no_index)?;
        let mut frontier: Vec<TreeNode> = root.children.unwrap_or_default();
        let mut out: Vec<TreeNode> = Vec::new();
        let mut guard = 0;
        while !frontier.is_empty() && guard < 400 {
            guard += 1;
            frontier.sort_by(|a, b| b.bytes.cmp(&a.bytes));
            let top = frontier.remove(0);
            if top.kind != "dir" {
                out.push(top);
            } else {
                let kids = self.0.engine.tree(Some(&top.path), 1).and_then(|t| t.children).unwrap_or_default();
                let biggest = kids.iter().filter(|k| k.kind == "dir").map(|k| k.bytes).max().unwrap_or(0);
                // A folder dominated by one child gets expanded; otherwise it's an answer.
                if biggest as f64 > top.bytes as f64 * 0.6 && out.len() + frontier.len() < limit * 2 {
                    frontier.extend(kids.into_iter().filter(|k| k.kind != "other"));
                } else {
                    let mut t = top;
                    t.children = None;
                    out.push(t);
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        out.extend(frontier.into_iter().map(|mut t| {
            t.children = None;
            t
        }));
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        out.truncate(limit);
        Ok(out)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<TreeNode>, String> {
        Ok(self.0.engine.search(query, limit))
    }

    fn clutter(&self) -> Result<Vec<Finding>, String> {
        let mut v = self.0.all_findings();
        v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        Ok(v)
    }

    fn explain(&self, path: &str) -> Result<Value, String> {
        let mut out = json!({ "path": path });
        if let Some(n) = self.0.engine.size_of(path) {
            out["bytes"] = json!(n.bytes);
            out["size"] = json!(disko_memory::fmt_bytes(n.bytes));
            out["files"] = json!(n.files);
            if let Some(t) = n.newest {
                out["last_used"] = json!(t);
            }
            if let Some(c) = n.clutter {
                out["category"] = json!(c);
            }
            if let Some(e) = n.error {
                out["error"] = json!(e);
            }
        } else if std::fs::symlink_metadata(path).is_err() {
            return Err(format!("{path} doesn't exist."));
        }
        // The finding covering this path (exact, inside, or containing it).
        let all = self.0.all_findings();
        let hit = all
            .iter()
            .find(|f| f.paths.iter().any(|p| p == path))
            .or_else(|| all.iter().find(|f| f.paths.iter().any(|p| path.starts_with(&format!("{p}/")))));
        if let Some(f) = hit {
            out["finding_id"] = json!(f.id);
            out["rule"] = json!(f.rule);
            out["category"] = json!(f.category);
            out["owner"] = json!(f.title);
            out["regrows"] = json!(f.regrows);
            out["safety"] = json!(f.safety);
            out["why"] = json!(f.why);
            if let Some(b) = &f.busy {
                out["busy"] = json!(b);
            }
        }
        Ok(out)
    }

    fn clean(&self, req: &CleanRequest) -> Result<CleanResult, String> {
        Ok(self.0.clean(&req.ids, &req.trash_ids, &req.paths))
    }

    fn apps(&self) -> Result<Value, String> {
        Ok(json!(disko_apps::apps()))
    }

    fn sessions(&self) -> Result<Value, String> {
        Ok(json!(disko_apps::sessions()))
    }

    fn rescan(&self) -> Result<(), String> {
        self.0.rescan();
        Ok(())
    }

    fn memory_changed(&self) {
        self.0.memory_changed();
    }
}
