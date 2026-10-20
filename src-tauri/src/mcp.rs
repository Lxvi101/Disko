//! `disko --mcp`: a small MCP server (JSON-RPC over stdio) that gives the assistant hands.
//!
//! Codex spawns it next to its own read-only sandbox. The tools can look around and move things
//! into Disko's quarantine, and restore them. There is deliberately no tool that deletes:
//! emptying the quarantine stays with the user.
use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const MAX_ITEMS: usize = 100;

fn tools() -> Value {
    json!([
        {
            "name": "disk_status",
            "description": "Free and total space on the home volume, plus how much is waiting in Disko's quarantine.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "quick_scan",
            "description": "Run Disko's Quick Clean sweep (a few seconds): build folders, node_modules, package caches, Xcode, Docker, Trash, logs, app caches, old installers. Returns groups by program with each item's path, size, whether it is safe to tick by default (`preselect`) and why it is locked (`blocked`). Docker and other tool-managed groups list a command instead of a path; you cannot run those, recommend them.",
            "inputSchema": { "type": "object", "properties": { "max_items_per_group": { "type": "integer", "minimum": 1, "maximum": 50 } } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "quarantine",
            "description": "Move files or folders into Disko's quarantine. This is reversible: the user can restore them, and only the user can empty the quarantine to actually free the space. Use absolute paths inside the home folder. Protected paths (credentials, system, cloud folders, app profiles and repositories) are refused. At most 100 items per call.",
            "inputSchema": {
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "reason"],
                            "properties": {
                                "path": { "type": "string", "description": "Absolute path" },
                                "reason": { "type": "string", "description": "One short sentence: why this is safe to remove" }
                            }
                        }
                    }
                }
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false }
        },
        {
            "name": "quarantine_list",
            "description": "What is currently in Disko's quarantine, batch by batch.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "quarantine_restore",
            "description": "Put a quarantined batch back where it came from. Use the batch `id` from quarantine_list or from a quarantine result.",
            "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } },
            "annotations": { "readOnlyHint": false, "destructiveHint": false }
        }
    ])
}

fn call(name: &str, args: &Value) -> Result<Value, String> {
    let home = home_dir();
    match name {
        "disk_status" => {
            let v = volume_info(&home);
            let held: u64 = list_quarantine_journals_sync()?.iter()
                .flat_map(|j| j.items.iter()).filter(|i| i.present).map(|i| i.size).sum();
            Ok(json!({ "free_bytes": v.available, "total_bytes": v.total, "free": fmt_bytes(v.available), "total": fmt_bytes(v.total), "in_quarantine": fmt_bytes(held) }))
        }
        "quick_scan" => {
            let max = args.get("max_items_per_group").and_then(Value::as_u64).unwrap_or(20) as usize;
            let report = quick_clean::discover(&home, &mut |_| {})?;
            let groups: Vec<Value> = report.groups.iter().map(|g| json!({
                "program": g.program, "group": g.title, "total": fmt_bytes(g.total), "after_cleanup": g.after,
                "managed_by_command": g.command.as_ref().map(|c| c.rsplit('/').next().unwrap_or(c).to_string()),
                "items": g.items.iter().take(max).map(|i| json!({
                    "path": if i.path.is_empty() { Value::Null } else { json!(i.path) }, "label": i.label, "size": fmt_bytes(i.total), "bytes": i.total,
                    "detail": i.detail, "preselect": i.preselect, "blocked": i.blocked,
                })).collect::<Vec<_>>(),
                "more_items": g.items.len().saturating_sub(max),
            })).collect();
            Ok(json!({ "groups": groups, "notes": report.notes }))
        }
        "quarantine" => {
            let items = args.get("items").and_then(Value::as_array).ok_or("`items` must be a list")?;
            if items.is_empty() { return Err("Nothing to quarantine".into()); }
            if items.len() > MAX_ITEMS { return Err(format!("At most {} items per call", MAX_ITEMS)); }
            let home_s = home.to_string_lossy().to_string();
            let mut accepted = vec![];
            let mut refused = vec![];
            for it in items {
                let path = it.get("path").and_then(Value::as_str).unwrap_or("").trim_end_matches('/').to_string();
                let (cat, why) = classify_path(&path, &home_s);
                // The assistant only moves things the rules call disposable or reviewable; app state,
                // repositories and profiles ("keep") need the user to act in the owning app.
                if !Path::new(&path).starts_with(&home) || cat == "protected" || cat == "keep" {
                    refused.push(json!({ "path": path, "why": format!("{} ({})", why, cat) }));
                    continue;
                }
                if let Err(e) = validate_cleanup_items(&[QuarantineInput { path: path.clone(), size: 0 }], &home) {
                    refused.push(json!({ "path": path, "why": e }));
                    continue;
                }
                accepted.push(path);
            }
            if accepted.is_empty() {
                return Ok(json!({ "moved": [], "refused": refused, "note": "Nothing was moved." }));
            }
            let sized: Vec<QuarantineInput> = accepted.iter().map(|p| QuarantineInput { path: p.clone(), size: quick_clean::du(Path::new(p)) }).collect();
            let r = safe_quarantine_items_sync(sized, &mut |_| {})?;
            let id = Path::new(&r.journal_path).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            Ok(json!({
                "batch_id": id, "moved": r.moved_items, "skipped_missing": r.skipped, "refused": refused,
                "bytes": r.total_bytes, "size": fmt_bytes(r.total_bytes),
                "note": "Moved to quarantine. Space is freed only when the user empties the quarantine; tell them that, and that they can restore the batch.",
            }))
        }
        "quarantine_list" => {
            let js = list_quarantine_journals_sync()?;
            Ok(json!({ "batches": js.iter().filter(|j| j.items.iter().any(|i| i.present)).map(|j| json!({
                "id": j.id, "created_at": j.created_at, "size": fmt_bytes(j.items.iter().filter(|i| i.present).map(|i| i.size).sum()),
                "items": j.items.iter().filter(|i| i.present).map(|i| json!({ "original": i.original, "size": fmt_bytes(i.size) })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>() }))
        }
        "quarantine_restore" => {
            let id = args.get("id").and_then(Value::as_str).ok_or("`id` is required")?;
            let j = list_quarantine_journals_sync()?.into_iter().find(|j| j.id == id).ok_or("No quarantine batch with that id")?;
            let n = restore_journal_sync(j.path)?;
            Ok(json!({ "restored_items": n }))
        }
        _ => Err(format!("Unknown tool {}", name)),
    }
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone(); // notifications get no reply
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "disko", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Disko's tools. You can inspect disk usage and quarantine items. You cannot delete anything; the user empties the quarantine.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            Ok(match call(name, &args) {
                Ok(v) => json!({ "content": [{ "type": "text", "text": v.to_string() }], "structuredContent": v, "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err(json!({ "code": -32601, "message": format!("Method not found: {}", method) })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() { continue; }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{}", r);
            let _ = out.flush();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_tools_and_has_no_delete() {
        let r = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).unwrap();
        let names: Vec<&str> = r.pointer("/result/tools").unwrap().as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"quarantine"));
        assert!(!names.iter().any(|n| n.contains("delete") || n.contains("purge") || n.contains("empty")));
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
    }

    #[test]
    fn quarantine_refuses_outside_home_and_keep() {
        let err = call("quarantine", &json!({ "items": [{ "path": "/etc/hosts", "reason": "x" }] })).unwrap();
        assert_eq!(err["moved"].as_array().unwrap().len(), 0);
        assert_eq!(err["refused"].as_array().unwrap().len(), 1);
    }
}
