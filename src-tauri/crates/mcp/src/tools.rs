//! Tool definitions and implementations. Outputs are compact JSON built for agents: sizes as
//! text and bytes, absolute paths, capped lists with `more` counts, and a `next` hint.

use crate::{CleanRequest, Server};
use disko_core::paths::is_protected;
use disko_core::{Action, Category, Finding, Safety, Source, TreeNode};
use disko_memory::routines::reject_reason;
use disko_memory::{fmt_bytes, now, Glob, LearnedRule, MemoryOp, Routine, RoutineSelect};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

const DAY: i64 = 86_400;

// ---------- definitions ----------

fn tool(name: &str, description: &str, props: Value, required: &[&str], read_only: bool, destructive: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": props, "required": required },
        "annotations": { "readOnlyHint": read_only, "destructiveHint": destructive, "openWorldHint": false },
    })
}

const CATS: &str = "build, cache, packages, developer, leftovers, old-versions, installers, logs, agent-data, containers, extras, forgotten, trash";

pub fn definitions() -> Value {
    let cat = json!({ "type": "string", "description": format!("One of: {}", CATS) });
    let cats = json!({ "type": "array", "items": { "type": "string" }, "description": format!("Categories: {}", CATS) });
    json!([
        tool("overview", "Disk at a glance: total/used/free, space a file walk can't see (snapshots, system), scan state, clutter totals by category, top findings, routines that are due. Call this first.", json!({}), &[], true, false),
        tool("largest", "Largest folders or files, flat and sorted. Fastest way to answer 'what's using space'.", json!({
            "under": { "type": "string", "description": "Absolute folder (or ~/…); default: whole disk" },
            "limit": { "type": "integer", "description": "Default 25, max 200" },
            "files_only": { "type": "boolean", "description": "Only files (e.g. big videos, disk images)" },
        }), &[], true, false),
        tool("tree", "Folder sizes below a path, largest first, a few levels deep.", json!({
            "path": { "type": "string", "description": "Absolute folder (or ~/…); default /" },
            "depth": { "type": "integer", "description": "Levels, default 2, max 5" },
            "limit": { "type": "integer", "description": "Children per folder, default 12, max 100" },
        }), &[], true, false),
        tool("search", "Find files/folders by name anywhere in the index (case-insensitive substring), with sizes.", json!({
            "query": { "type": "string" },
            "limit": { "type": "integer", "description": "Default 30, max 300" },
        }), &["query"], true, false),
        tool("clutter", "What Disko suggests removing, largest first. Each finding has an id for `clean` and says how `clean` removes it (`trash`, or the owning tool's `command`). Kept items are hidden.", json!({
            "category": { "description": "A category or list of categories", "anyOf": [cat.clone(), cats.clone()] },
            "safety": { "type": "string", "enum": ["safe", "review"], "description": "safe = rebuildable, regrows; review = the user's call. Through this server both go to the Trash." },
            "query": { "type": "string", "description": "Substring of title, rule or path" },
            "min_bytes": { "type": "integer", "description": "Default 1000000 (1 MB); 0 for everything" },
            "limit": { "type": "integer", "description": "Default 40, max 500" },
            "include_kept": { "type": "boolean", "description": "Also list findings the keep list protects (marked kept)" },
        }), &[], true, false),
        tool("explain", "Everything Disko knows about one path: size, last used, category, rule, owner app/tool, regrows, keep status, learned rules, whether it's protected.", json!({
            "path": { "type": "string", "description": "Absolute path or ~/…" },
        }), &["path"], true, false),
        tool("clean", "Move findings (by id) and/or paths to the macOS Trash (Undo: Finder › Put Back, or Disko's Undo). Findings managed by a tool run that tool's own cleanup command instead. Never deletes anything permanently: emptying the Trash or permanent deletion is the user's call, in Disko. The keep list, protected paths, in-use and admin-only items are skipped. Returns what was moved/freed.", json!({
            "ids": { "type": "array", "items": { "type": "string" }, "description": "Finding ids from `clutter`" },
            "paths": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths; moved to the Trash" },
            "dry_run": { "type": "boolean", "description": "Only show what would happen" },
        }), &[], false, true),
        tool("apps", "Installed apps with app size, data size and last use, largest first.", json!({
            "query": { "type": "string", "description": "Substring of name or bundle id" },
            "limit": { "type": "integer", "description": "Default 30, max 500" },
            "details": { "type": "boolean", "description": "Include data paths" },
        }), &[], true, false),
        tool("sessions", "Coding-agent data (Claude Code, Codex, T3 Code, Cursor …) per tool and project, with sizes and last use.", json!({
            "tool": { "type": "string", "description": "Only this tool (e.g. claude-code); also lists its sessions" },
            "limit": { "type": "integer", "description": "Projects per tool, default 10" },
        }), &[], true, false),
        tool("rescan", "Start a fresh whole-disk scan. Results stay usable and update live while it runs.", json!({}), &[], false, false),
        tool("memory", "Disko's memory: keep list, learned rules, routines (with due status), community pack and queued shared rules.", json!({}), &[], true, false),
        tool("keep", "Keep something forever: Disko never suggests or cleans it (or anything inside it). Use when the user says to keep something.", json!({
            "pattern": { "type": "string", "description": "Absolute path or ~/ glob (`*`, `**`)" },
            "note": { "type": "string", "description": "Why, in the user's words" },
        }), &["pattern"], false, false),
        tool("unkeep", "Remove a keep entry an agent added (by id or exact pattern). Entries the user added can only be removed by the user, in Disko.", json!({ "id": { "type": "string" } }), &["id"], false, false),
        tool("add_rule", "Teach Disko a new source of clutter it missed (a tool cache, a build output folder). Its findings show up in `clutter` as review items: `clean` moves them to the Trash. Agent rules are never deleted permanently and never run in routines; the user can promote one in Disko. Can't change the user's own rules. Returns what it matches right now.", json!({
            "pattern": { "type": "string", "description": "Glob inside the home folder: ~/.turbo/cache, ~/Library/Caches/com.vendor.app, ~/**/.dart_tool. Not allowed: outside home, .git folders, ~/Library except Caches/<app> and Logs/<app>, credentials (.ssh, .aws, …). With `**`, the last part must be a known build/cache folder name (node_modules, target, .build, dist, .next, .dart_tool, …)." },
            "title": { "type": "string", "description": "Short human name: 'Turbo cache'" },
            "category": cat,
            "why": { "type": "string", "description": "One sentence of evidence: what creates it and why removing it is fine" },
            "regrows": { "type": "boolean", "description": "Comes back by itself / the tool recreates it" },
            "safety": { "type": "string", "enum": ["safe", "review"], "description": "Your assessment (safe = rebuildable). Agent rules are stored as review either way; only the user can make a rule safe." },
            "share": { "type": "boolean", "description": "Share anonymized with the community pack (only generic sources)" },
            "id": { "type": "string", "description": "Optional id; reuse an id to update a rule" },
            "dry_run": { "type": "boolean", "description": "Validate and preview matches without saving" },
        }), &["pattern", "title", "category"], false, false),
        tool("remove_rule", "Forget a learned rule (by id).", json!({ "id": { "type": "string" } }), &["id"], false, false),
        tool("set_routine", "Create or change a routine ('clean X every N days'). With an existing id, only the fields given change (e.g. {id:'build-artifacts', enabled:true}). Only create or enable routines the user asked for.", json!({
            "id": { "type": "string" },
            "title": { "type": "string" },
            "categories": cats,
            "rules": { "type": "array", "items": { "type": "string" }, "description": "Finding rule ids, prefixes, or the user's learned rule ids (agent and community rules never run in routines)" },
            "every_days": { "type": "integer" },
            "min_age_days": { "type": "integer", "description": "Only items untouched this long" },
            "enabled": { "type": "boolean" },
            "review": { "type": "boolean", "description": "Also take review items (to the Trash). Only if the user explicitly wants that." },
        }), &[], false, false),
        tool("remove_routine", "Delete a routine (by id).", json!({ "id": { "type": "string" } }), &["id"], false, false),
        tool("run_routine", "Run a routine now (or, without id, every routine that's due). Run from here, its items go to the Trash (or run the owning tool's cleanup); nothing is deleted permanently. Routines only take findings from Disko's built-in and the user's own rules. Returns what was moved/freed and what was skipped and why.", json!({
            "id": { "type": "string" },
            "dry_run": { "type": "boolean" },
        }), &[], false, true),
    ])
}

// ---------- helpers ----------

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty())
}
fn n(a: &Value, k: &str, default: u64, max: u64) -> u64 {
    a.get(k).and_then(Value::as_u64).unwrap_or(default).min(max)
}
fn b(a: &Value, k: &str) -> Option<bool> {
    a.get(k).and_then(Value::as_bool)
}
fn strs(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::String(x)) if !x.trim().is_empty() => vec![x.trim().to_string()],
        Some(Value::Array(v)) => v.iter().filter_map(Value::as_str).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    }
}

/// `~/x` → absolute; strips trailing slashes.
fn abs_path(p: &str) -> Result<String, String> {
    let p = p.trim();
    let out = if p == "~" {
        disko_core::paths::home().to_string_lossy().to_string()
    } else if let Some(r) = p.strip_prefix("~/") {
        disko_core::paths::home().join(r).to_string_lossy().to_string()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        return Err(format!("`{}` must be an absolute path (or ~/…).", p));
    };
    let t = out.trim_end_matches('/');
    Ok(if t.is_empty() { "/".into() } else { t.to_string() })
}

fn category(x: &str) -> Result<Category, String> {
    serde_json::from_value(json!(x)).map_err(|_| format!("Unknown category `{}`. Use one of: {}.", x, CATS))
}

fn safety(x: &str) -> Result<Safety, String> {
    serde_json::from_value(json!(x)).map_err(|_| "safety must be `safe` or `review`.".to_string())
}

fn kebab<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn idle_days(t: Option<i64>, now: i64) -> Option<i64> {
    t.filter(|t| *t > 0).map(|t| ((now - t).max(0)) / DAY)
}

fn sized(m: &mut Map<String, Value>, bytes: u64) {
    m.insert("size".into(), json!(fmt_bytes(bytes)));
    m.insert("bytes".into(), json!(bytes));
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn action_word(a: &Action) -> String {
    match a {
        Action::Delete => "delete".into(),
        Action::Trash => "trash".into(),
        Action::Command { label, .. } => format!("command: {}", label),
    }
}

/// What `clean` (and `run_routine`) through this server does with a finding: the owning tool's
/// command, or the Trash. Agents never delete permanently.
fn mcp_how(f: &Finding) -> String {
    match &f.action {
        Action::Command { .. } => action_word(&f.action),
        _ => "trash".into(),
    }
}

/// Split findings into tool commands (`ids`) and everything else (`trash_ids`).
fn trash_request(fs: &[&Finding]) -> CleanRequest {
    let mut req = CleanRequest::default();
    for f in fs {
        if matches!(f.action, Action::Command { .. }) {
            req.ids.push(f.id.clone());
        } else {
            req.trash_ids.push(f.id.clone());
        }
    }
    req
}

fn finding_row(f: &Finding, now: i64) -> Value {
    let mut m = obj(json!({ "id": f.id, "title": f.title }));
    if let Some(d) = &f.detail {
        m.insert("detail".into(), json!(d));
    }
    m.insert("category".into(), kebab(f.category));
    sized(&mut m, f.bytes);
    m.insert("safety".into(), kebab(f.safety));
    m.insert("action".into(), json!(mcp_how(f)));
    if f.regrows {
        m.insert("regrows".into(), json!(true));
    }
    if let Some(d) = idle_days(f.last_used, now) {
        m.insert("idle_days".into(), json!(d));
    }
    m.insert("paths".into(), json!(f.paths.iter().take(3).collect::<Vec<_>>()));
    if f.paths.len() > 3 {
        m.insert("more_paths".into(), json!(f.paths.len() - 3));
    }
    m.insert("why".into(), json!(f.why));
    if let Some(bz) = &f.busy {
        m.insert("busy".into(), json!(bz));
    }
    if f.admin {
        m.insert("admin".into(), json!(true));
    }
    if f.source != Source::Builtin {
        m.insert("source".into(), kebab(f.source));
    }
    Value::Object(m)
}

fn node_row(t: &TreeNode, now: i64) -> Map<String, Value> {
    let mut m = obj(json!({ "path": t.path }));
    if t.kind != "dir" {
        m.insert("kind".into(), json!(t.kind));
    }
    sized(&mut m, t.bytes);
    if t.kind == "dir" {
        m.insert("files".into(), json!(t.files));
    }
    if let Some(d) = idle_days(t.newest, now) {
        m.insert("idle_days".into(), json!(d));
    }
    if let Some(c) = t.clutter {
        m.insert("clutter".into(), kebab(c));
    }
    if let Some(e) = &t.error {
        m.insert("error".into(), json!(e));
    }
    m
}

fn tree_row(t: &TreeNode, now: i64, limit: usize) -> Value {
    let mut m = node_row(t, now);
    if let Some(ch) = &t.children {
        let rows: Vec<Value> = ch.iter().take(limit).map(|c| tree_row(c, now, limit)).collect();
        m.insert("children".into(), json!(rows));
        if ch.len() > limit {
            let rest: u64 = ch.iter().skip(limit).map(|c| c.bytes).sum();
            m.insert("more".into(), json!({ "count": ch.len() - limit, "size": fmt_bytes(rest), "bytes": rest }));
        }
    }
    Value::Object(m)
}

fn scan_note(srv: &Server) -> Option<Value> {
    let f = srv.backend().freshness();
    if f.scanning {
        Some(json!(format!("Scan running ({:.0}%): results are partial and keep improving.", f.fraction * 100.0)))
    } else if f.phase == "none" {
        Some(json!("No scan yet; call rescan."))
    } else {
        None
    }
}

fn with_scan(mut m: Map<String, Value>, srv: &Server) -> Value {
    if let Some(n) = scan_note(srv) {
        m.insert("scan".into(), n);
    }
    Value::Object(m)
}

// ---------- calls ----------

pub fn call(srv: &Server, name: &str, a: &Value) -> Result<Value, String> {
    let t = now();
    match name {
        "overview" => overview(srv, t),
        "largest" => {
            let under = s(a, "under").map(abs_path).transpose()?;
            let limit = n(a, "limit", 25, 200) as usize;
            let rows = srv.backend().largest(under.as_deref(), limit, b(a, "files_only").unwrap_or(false))?;
            let total: u64 = rows.iter().map(|r| r.bytes).sum();
            let mut m = Map::new();
            m.insert("items".into(), json!(rows.iter().map(|r| Value::Object(node_row(r, t))).collect::<Vec<_>>()));
            m.insert("listed".into(), json!(fmt_bytes(total)));
            m.insert("next".into(), json!("tree(path) to drill in, explain(path) for one item, clutter for removable things."));
            Ok(with_scan(m, srv))
        }
        "tree" => {
            let path = s(a, "path").map(abs_path).transpose()?;
            let depth = n(a, "depth", 2, 5).max(1) as u32;
            let limit = n(a, "limit", 12, 100).max(1) as usize;
            let root = srv.backend().tree(path.as_deref(), depth)?;
            let mut m = obj(tree_row(&root, t, limit));
            if let Some(n) = scan_note(srv) {
                m.insert("scan".into(), n);
            }
            Ok(Value::Object(m))
        }
        "search" => {
            let q = s(a, "query").ok_or("`query` is required.")?;
            let limit = n(a, "limit", 30, 300) as usize;
            let rows = srv.backend().search(q, limit)?;
            let mut m = Map::new();
            m.insert("count".into(), json!(rows.len()));
            m.insert("items".into(), json!(rows.iter().map(|r| Value::Object(node_row(r, t))).collect::<Vec<_>>()));
            Ok(with_scan(m, srv))
        }
        "clutter" => clutter(srv, a, t),
        "explain" => explain(srv, a),
        "clean" => clean(srv, a),
        "apps" => apps(srv, a, t),
        "sessions" => sessions(srv, a, t),
        "rescan" => {
            srv.backend().rescan()?;
            Ok(json!({ "started": true, "next": "overview/clutter work right away and fill in as the scan runs." }))
        }
        "memory" => memory(srv, t),
        "keep" => keep(srv, a),
        "unkeep" => {
            let id = s(a, "id").ok_or("`id` is required.")?;
            srv.store().apply(MemoryOp::Unkeep { id: id.to_string(), source: Some(Source::Agent) })?;
            srv.backend().memory_changed();
            Ok(json!({ "removed": id }))
        }
        "add_rule" => add_rule(srv, a),
        "remove_rule" => {
            let id = s(a, "id").ok_or("`id` is required.")?;
            srv.store().apply(MemoryOp::RemoveRule { id: id.to_string() })?;
            srv.backend().memory_changed();
            Ok(json!({ "removed": id }))
        }
        "set_routine" => set_routine(srv, a, t),
        "remove_routine" => {
            let id = s(a, "id").ok_or("`id` is required.")?;
            srv.store().apply(MemoryOp::RemoveRoutine { id: id.to_string() })?;
            srv.backend().memory_changed();
            Ok(json!({ "removed": id }))
        }
        "run_routine" => run_routine(srv, a, t),
        _ => Err(format!("Unknown tool `{}`.", name)),
    }
}

fn overview(srv: &Server, t: i64) -> Result<Value, String> {
    let be = srv.backend();
    let v = be.volume()?;
    let fr = be.freshness();
    let findings = be.clutter().unwrap_or_default();
    let mut store = srv.store();
    let keep = store.keep().clone();
    let mem = store.memory();
    drop(store);
    let live: Vec<&Finding> = findings.iter().filter(|f| keep.finding_kept_by(f).is_none()).collect();
    let mut by_cat: BTreeMap<Category, (usize, u64, u64)> = BTreeMap::new();
    for f in &live {
        let e = by_cat.entry(f.category).or_default();
        e.0 += 1;
        if f.safety == Safety::Safe {
            e.1 += f.bytes;
        } else {
            e.2 += f.bytes;
        }
    }
    let mut cats: Vec<(Category, (usize, u64, u64))> = by_cat.into_iter().collect();
    cats.sort_by_key(|(_, (_, s, r))| std::cmp::Reverse(s + r));
    let safe: u64 = live.iter().filter(|f| f.safety == Safety::Safe).map(|f| f.bytes).sum();
    let review: u64 = live.iter().filter(|f| f.safety == Safety::Review).map(|f| f.bytes).sum();
    let mut top: Vec<&&Finding> = live.iter().collect();
    top.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    let routines: Vec<Value> = mem
        .routines
        .iter()
        .map(|r| {
            json!({ "id": r.id, "title": r.title, "enabled": r.enabled, "due": r.due(t),
                    "would_free": fmt_bytes(disko_memory::select(&findings, r, t, &keep).iter().map(|f| f.bytes).sum()) })
        })
        .collect();
    let mut scan = obj(json!({ "phase": fr.phase, "files": fr.files }));
    if fr.scanning {
        scan.insert("progress".into(), json!(format!("{:.0}%", fr.fraction * 100.0)));
    }
    if let Some(a) = fr.age_s {
        scan.insert("age".into(), json!(if a < 120 { format!("{}s", a) } else if a < 7200 { format!("{}m", a / 60) } else { format!("{}h", a / 3600) }));
    }
    Ok(json!({
        "disk": { "name": v.name, "total": fmt_bytes(v.total), "used": fmt_bytes(v.used), "free": fmt_bytes(v.free),
                  "free_bytes": v.free, "purgeable": fmt_bytes(v.purgeable), "indexed": fmt_bytes(v.scanned) },
        "hidden": v.hidden.iter().filter(|h| h.bytes > 0).map(|h| json!({ "what": h.label, "size": fmt_bytes(h.bytes), "why": h.why })).collect::<Vec<_>>(),
        "scan": scan,
        "clutter": {
            "safe": fmt_bytes(safe), "review": fmt_bytes(review), "findings": live.len(),
            "by_category": cats.iter().map(|(c, (n, s, r))| json!({ "category": kebab(*c), "count": n, "safe": fmt_bytes(*s), "review": fmt_bytes(*r) })).collect::<Vec<_>>(),
            "top": top.iter().take(5).map(|f| json!({ "id": f.id, "title": f.title, "size": fmt_bytes(f.bytes), "safety": kebab(f.safety) })).collect::<Vec<_>>(),
        },
        "memory": { "keep": mem.keep.len(), "rules": mem.rules.len(), "community_rules": mem.community.rules, "routines": routines },
        "next": "clutter(category|safety) for ids → clean(ids). largest/tree for where space went.",
    }))
}

fn clutter(srv: &Server, a: &Value, t: i64) -> Result<Value, String> {
    let cats: Vec<Category> = strs(a, "category").iter().map(|c| category(c)).collect::<Result<_, _>>()?;
    let saf = s(a, "safety").map(safety).transpose()?;
    let q = s(a, "query").map(str::to_lowercase);
    let min = a.get("min_bytes").and_then(Value::as_u64).unwrap_or(1_000_000);
    let limit = n(a, "limit", 40, 500) as usize;
    let include_kept = b(a, "include_kept").unwrap_or(false);
    let all = srv.backend().clutter()?;
    let keep = srv.store().keep().clone();
    let mut kept_hidden = (0usize, 0u64);
    let mut rows: Vec<(u64, Value)> = vec![];
    let (mut total_n, mut total_b) = (0usize, 0u64);
    for f in &all {
        if (!cats.is_empty() && !cats.contains(&f.category)) || saf.is_some_and(|x| x != f.safety) || f.bytes < min {
            continue;
        }
        if let Some(q) = &q {
            let hay = format!("{} {} {} {}", f.title, f.detail.as_deref().unwrap_or(""), f.rule, f.paths.join(" ")).to_lowercase();
            if !hay.contains(q) {
                continue;
            }
        }
        let kept = keep.finding_kept_by(f);
        if kept.is_some() && !include_kept {
            kept_hidden.0 += 1;
            kept_hidden.1 += f.bytes;
            continue;
        }
        let mut row = finding_row(f, t);
        if let (Some(k), Value::Object(m)) = (kept, &mut row) {
            m.insert("kept".into(), json!(k.pattern));
        }
        total_n += 1;
        total_b += f.bytes;
        rows.push((f.bytes, row));
    }
    rows.sort_by_key(|(b, _)| std::cmp::Reverse(*b));
    let shown: Vec<Value> = rows.into_iter().take(limit).map(|(_, v)| v).collect();
    let mut m = obj(json!({ "count": total_n, "size": fmt_bytes(total_b), "bytes": total_b }));
    if shown.len() < total_n {
        m.insert("shown".into(), json!(shown.len()));
    }
    m.insert("findings".into(), json!(shown));
    if kept_hidden.0 > 0 {
        m.insert("kept_hidden".into(), json!({ "count": kept_hidden.0, "size": fmt_bytes(kept_hidden.1) }));
    }
    m.insert("next".into(), json!("clean(ids) removes them (dry_run to preview); explain(path) for evidence; keep(pattern) if the user wants one kept."));
    Ok(with_scan(m, srv))
}

fn explain(srv: &Server, a: &Value) -> Result<Value, String> {
    let p = abs_path(s(a, "path").ok_or("`path` is required.")?)?;
    let mut m = obj(srv.backend().explain(&p).unwrap_or_else(|e| json!({ "note": e })));
    m.insert("path".into(), json!(p));
    if let Some(bytes) = m.get("bytes").and_then(Value::as_u64) {
        m.entry("size").or_insert(json!(fmt_bytes(bytes)));
    }
    if let Some(lu) = m.get("last_used").and_then(Value::as_i64) {
        if let Some(d) = idle_days(Some(lu), now()) {
            m.insert("idle_days".into(), json!(d));
        }
    }
    if is_protected(Path::new(&p)) {
        m.insert("protected".into(), json!("Disko never removes this."));
    }
    let mut store = srv.store();
    if let Some(k) = store.keep().kept_by(&p) {
        m.insert("kept".into(), json!({ "id": k.id, "pattern": k.pattern, "note": k.note }));
    }
    let home = store.home().to_path_buf();
    let rules = store.active_rules();
    drop(store);
    let matcher = disko_memory::LearnedMatcher::new(&rules, &home, 0);
    if let Some(r) = matcher.covering(&p) {
        m.insert(
            "learned_rule".into(),
            json!({ "id": r.id, "title": r.title, "pattern": r.pattern, "source": kebab(r.source), "safety": kebab(r.safety) }),
        );
    }
    if !m.contains_key("rule") && !m.contains_key("learned_rule") && !m.contains_key("protected") {
        m.insert("next".into(), json!("Not a known kind of clutter. If it's a rebuildable source others will have too, add_rule it."));
    }
    Ok(Value::Object(m))
}

fn skip(list: &mut Vec<Value>, what: &str, why: &str) {
    list.push(json!({ "item": what, "why": why }));
}

fn clean(srv: &Server, a: &Value) -> Result<Value, String> {
    let ids = strs(a, "ids");
    let paths: Vec<String> = strs(a, "paths").iter().map(|p| abs_path(p)).collect::<Result<_, _>>()?;
    if ids.is_empty() && paths.is_empty() {
        return Err("Give `ids` (from clutter) and/or `paths`.".into());
    }
    if ids.len() + paths.len() > 500 {
        return Err("At most 500 items per call.".into());
    }
    // Through MCP nothing is deleted permanently: findings go to the Trash (or run their tool's
    // own cleanup command), whatever their action or a `trash` flag say.
    let dry = b(a, "dry_run").unwrap_or(false);
    let findings = srv.backend().clutter()?;
    let keep = srv.store().keep().clone();
    let mut req = CleanRequest::default();
    let mut skipped: Vec<Value> = vec![];
    let mut plan: Vec<Value> = vec![];
    let mut planned_bytes = 0u64;

    let mut take = |f: &Finding, req: &mut CleanRequest, skipped: &mut Vec<Value>, plan: &mut Vec<Value>| {
        if let Some(k) = keep.finding_kept_by(f) {
            return skip(skipped, &f.id, &format!("on the keep list ({})", k.pattern));
        }
        if let Some(bz) = &f.busy {
            return skip(skipped, &f.id, &format!("in use: {}", bz));
        }
        if f.admin {
            return skip(skipped, &f.id, "needs an administrator password; ask the user to clean it in Disko");
        }
        if let Some(p) = f.paths.iter().find(|p| is_protected(Path::new(p))) {
            return skip(skipped, &f.id, &format!("{} is protected", p));
        }
        let how = mcp_how(f);
        if matches!(f.action, Action::Command { .. }) {
            req.ids.push(f.id.clone());
        } else {
            req.trash_ids.push(f.id.clone());
        }
        planned_bytes += f.bytes;
        plan.push(json!({ "id": f.id, "title": f.title, "size": fmt_bytes(f.bytes), "how": how }));
    };

    for id in &ids {
        match findings.iter().find(|f| &f.id == id) {
            Some(f) => take(f, &mut req, &mut skipped, &mut plan),
            None => skip(&mut skipped, id, "unknown id (already removed, or call clutter again)"),
        }
    }
    for p in &paths {
        // A path that is exactly a finding is cleaned as that finding (to the Trash, or its tool's command).
        if let Some(f) = findings.iter().find(|f| f.paths.len() == 1 && &f.paths[0] == p) {
            if !req.ids.contains(&f.id) && !req.trash_ids.contains(&f.id) {
                take(f, &mut req, &mut skipped, &mut plan);
            }
            continue;
        }
        if is_protected(Path::new(p)) {
            skip(&mut skipped, p, "protected; Disko never removes it");
        } else if let Some(k) = keep.kept_by(p) {
            skip(&mut skipped, p, &format!("on the keep list ({})", k.pattern));
        } else if std::fs::symlink_metadata(p).is_err() {
            skip(&mut skipped, p, "doesn't exist");
        } else {
            req.paths.push(p.clone());
            plan.push(json!({ "path": p, "how": "trash" }));
        }
    }

    if dry || (req.ids.is_empty() && req.trash_ids.is_empty() && req.paths.is_empty()) {
        let mut m = obj(json!({ "dry_run": dry, "would_clean": plan, "known_size": fmt_bytes(planned_bytes) }));
        if !skipped.is_empty() {
            m.insert("skipped".into(), json!(skipped));
        }
        if !dry {
            m.insert("note".into(), json!("Nothing to clean."));
        }
        return Ok(Value::Object(m));
    }

    let r = srv.backend().clean(&req)?;
    for s in &r.skipped {
        skip(&mut skipped, &s.path, &s.error);
    }
    let mut m = obj(json!({ "freed": fmt_bytes(r.freed), "freed_bytes": r.freed, "removed": r.removed.len() }));
    if !r.trashed.is_empty() {
        m.insert(
            "trashed".into(),
            json!(r.trashed.iter().take(50).map(|x| json!({ "from": x.from, "to": x.to, "size": fmt_bytes(x.bytes) })).collect::<Vec<_>>()),
        );
        m.insert("undo".into(), json!("Trashed items can be put back (Finder › Put Back, or Disko's Undo). Space is only freed for good when the user empties the Trash."));
    }
    if !r.failed.is_empty() {
        m.insert("failed".into(), json!(r.failed.iter().map(|f| json!({ "path": f.path, "error": f.error })).collect::<Vec<_>>()));
    }
    if !skipped.is_empty() {
        m.insert("skipped".into(), json!(skipped));
    }
    Ok(Value::Object(m))
}

fn apps(srv: &Server, a: &Value, t: i64) -> Result<Value, String> {
    let v = srv.backend().apps()?;
    let list = v.as_array().cloned().unwrap_or_default();
    let q = s(a, "query").map(str::to_lowercase);
    let limit = n(a, "limit", 30, 500) as usize;
    let details = b(a, "details").unwrap_or(false);
    let num = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
    let mut rows: Vec<(u64, Value)> = list
        .iter()
        .filter(|x| {
            q.as_ref().is_none_or(|q| {
                let hay = format!("{} {}", x.get("name").and_then(Value::as_str).unwrap_or(""), x.get("bundle_id").and_then(Value::as_str).unwrap_or(""));
                hay.to_lowercase().contains(q)
            })
        })
        .map(|x| {
            let (ab, db) = (num(x, "app_bytes"), num(x, "data_bytes"));
            let mut m = obj(json!({ "name": x.get("name"), "path": x.get("path") }));
            if let Some(bid) = x.get("bundle_id").filter(|v| !v.is_null()) {
                m.insert("bundle_id".into(), bid.clone());
            }
            if let Some(ver) = x.get("version").filter(|v| !v.is_null()) {
                m.insert("version".into(), ver.clone());
            }
            m.insert("size".into(), json!(fmt_bytes(ab + db)));
            m.insert("bytes".into(), json!(ab + db));
            m.insert("app".into(), json!(fmt_bytes(ab)));
            m.insert("data".into(), json!(fmt_bytes(db)));
            if let Some(d) = idle_days(x.get("last_used").and_then(Value::as_i64), t) {
                m.insert("idle_days".into(), json!(d));
            }
            if x.get("running").and_then(Value::as_bool) == Some(true) {
                m.insert("running".into(), json!(true));
            }
            if x.get("apple").and_then(Value::as_bool) == Some(true) {
                m.insert("apple".into(), json!(true));
            }
            if details {
                if let Some(dp) = x.get("data_paths") {
                    m.insert("data_paths".into(), dp.clone());
                }
            }
            (ab + db, Value::Object(m))
        })
        .collect();
    rows.sort_by_key(|(b, _)| std::cmp::Reverse(*b));
    let count = rows.len();
    Ok(json!({
        "count": count,
        "apps": rows.into_iter().take(limit).map(|(_, v)| v).collect::<Vec<_>>(),
        "next": "Uninstalling apps is done by the user in Disko. Leftovers of removed apps are in clutter(category: leftovers).",
    }))
}

fn sessions(srv: &Server, a: &Value, t: i64) -> Result<Value, String> {
    let v = srv.backend().sessions()?;
    let only = s(a, "tool");
    let limit = n(a, "limit", 10, 200) as usize;
    let num = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
    let mut tools_out = vec![];
    for tool in v.as_array().cloned().unwrap_or_default() {
        let id = tool.get("tool").and_then(Value::as_str).unwrap_or("");
        if only.is_some_and(|o| o != id) {
            continue;
        }
        let mut projects: Vec<Value> = tool.get("projects").and_then(Value::as_array).cloned().unwrap_or_default();
        projects.sort_by_key(|p| std::cmp::Reverse(num(p, "bytes")));
        let rows: Vec<Value> = projects
            .iter()
            .take(limit)
            .map(|p| {
                let mut m = obj(json!({ "name": p.get("name") }));
                if let Some(path) = p.get("path").filter(|v| !v.is_null()) {
                    m.insert("path".into(), path.clone());
                }
                sized(&mut m, num(p, "bytes"));
                if let Some(d) = idle_days(p.get("last_used").and_then(Value::as_i64), t) {
                    m.insert("idle_days".into(), json!(d));
                }
                let ss = p.get("sessions").and_then(Value::as_array).cloned().unwrap_or_default();
                m.insert("sessions".into(), json!(ss.len()));
                if only.is_some() {
                    let mut ss = ss;
                    ss.sort_by_key(|x| std::cmp::Reverse(num(x, "bytes")));
                    m.insert(
                        "session_list".into(),
                        json!(ss
                            .iter()
                            .take(20)
                            .map(|x| json!({ "id": x.get("id"), "title": x.get("title"), "size": fmt_bytes(num(x, "bytes")),
                                             "idle_days": idle_days(x.get("last_used").and_then(Value::as_i64), t), "paths": x.get("paths") }))
                            .collect::<Vec<_>>()),
                    );
                }
                Value::Object(m)
            })
            .collect();
        let mut m = obj(json!({ "tool": id, "name": tool.get("name") }));
        sized(&mut m, num(&tool, "bytes"));
        m.insert("projects".into(), json!(rows));
        if projects.len() > limit {
            m.insert("more_projects".into(), json!(projects.len() - limit));
        }
        tools_out.push(Value::Object(m));
    }
    Ok(json!({ "tools": tools_out, "next": "Removable agent data is in clutter(category: agent-data); clean by id or trash session paths." }))
}

fn memory(srv: &Server, t: i64) -> Result<Value, String> {
    let mut store = srv.store();
    let m = store.memory();
    let outbox = store.outbox();
    drop(store);
    Ok(json!({
        "keep": m.keep.iter().map(|k| json!({ "id": k.id, "pattern": k.pattern, "note": k.note, "source": kebab(k.source) })).collect::<Vec<_>>(),
        "rules": m.rules.iter().map(|r| json!({ "id": r.id, "pattern": r.pattern, "title": r.title, "category": kebab(r.category),
            "safety": kebab(r.safety), "regrows": r.regrows, "shared": r.shared, "source": kebab(r.source) })).collect::<Vec<_>>(),
        "routines": m.routines.iter().map(|r| routine_row(r, t)).collect::<Vec<_>>(),
        "community": { "enabled": m.community.enabled, "rules": m.community.rules, "queued_submissions": outbox.iter().map(|s| &s.pattern).collect::<Vec<_>>() },
    }))
}

fn routine_row(r: &Routine, t: i64) -> Value {
    let mut m = obj(json!({ "id": r.id, "title": r.title, "select": r.select, "every_days": r.every_days,
                            "min_age_days": r.min_age_days, "enabled": r.enabled, "due": r.due(t) }));
    if r.review {
        m.insert("review".into(), json!(true));
    }
    if let Some(d) = idle_days(r.last_run, t) {
        m.insert("last_run_days_ago".into(), json!(d));
    }
    if let Some(f) = r.last_freed {
        m.insert("last_freed".into(), json!(fmt_bytes(f)));
    }
    Value::Object(m)
}

fn keep(srv: &Server, a: &Value) -> Result<Value, String> {
    let pattern = s(a, "pattern").ok_or("`pattern` is required.")?.to_string();
    let note = s(a, "note").map(str::to_string);
    let mem = srv.store().apply(MemoryOp::Keep { pattern: pattern.clone(), note, source: Some(Source::Agent) })?;
    srv.backend().memory_changed();
    let p = pattern.trim_end_matches('/');
    let entry = mem.keep.iter().find(|k| k.pattern == p).cloned();
    let keep = srv.store().keep().clone();
    let hidden: Vec<Finding> = srv.backend().clutter().unwrap_or_default().into_iter().filter(|f| keep.finding_kept_by(f).is_some()).collect();
    Ok(json!({
        "kept": entry.map(|e| json!({ "id": e.id, "pattern": e.pattern })),
        "now_protects": { "findings": hidden.len(), "size": fmt_bytes(hidden.iter().map(|f| f.bytes).sum()) },
    }))
}

fn add_rule(srv: &Server, a: &Value) -> Result<Value, String> {
    let mut rule = LearnedRule {
        id: s(a, "id").unwrap_or("").to_string(),
        pattern: s(a, "pattern").ok_or("`pattern` is required.")?.trim_end_matches('/').to_string(),
        category: category(s(a, "category").ok_or("`category` is required.")?)?,
        title: s(a, "title").ok_or("`title` is required.")?.to_string(),
        why: s(a, "why").unwrap_or("").to_string(),
        regrows: b(a, "regrows").unwrap_or(false),
        safety: s(a, "safety").map(safety).transpose()?.unwrap_or(Safety::Review),
        source: Source::Agent,
        added: 0,
        shared: b(a, "share").unwrap_or(false),
    };
    let asked_safe = rule.safety == Safety::Safe;
    // Agent rules are always review (to the Trash, never in routines); see disko_memory::rules.
    disko_memory::rules::normalize_rule(&mut rule);
    let home = srv.store().home().to_path_buf();
    disko_memory::validate_rule(&rule, &home)?;
    let keep = srv.store().keep().clone();
    let glob = Glob::new(&rule.pattern, &home)?;
    // Preview: on disk for plain patterns, from the index for `**` ones.
    let matched: Vec<(String, u64)> = if glob.has_globstar() {
        let name = rule.pattern.rsplit('/').next().unwrap_or("");
        srv.backend()
            .search(name, 500)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| glob.matches(&n.path) && !disko_memory::rules::path_off_limits(Path::new(&n.path), &home))
            .map(|n| (n.path, n.bytes))
            .collect()
    } else {
        glob.expand(200)
            .into_iter()
            .filter(|p| !is_protected(p) && !disko_memory::rules::path_off_limits(p, &home))
            .map(|p| {
                let bytes = disko_memory::rules::measure(&p).0;
                (p.to_string_lossy().to_string(), bytes)
            })
            .collect()
    };
    let total: u64 = matched.iter().map(|m| m.1).sum();
    let kept = matched.iter().filter(|m| keep.is_kept(&m.0)).count();
    let preview = json!({
        "count": matched.len(), "size": fmt_bytes(total), "bytes": total,
        "sample": matched.iter().take(5).map(|m| json!({ "path": m.0, "size": fmt_bytes(m.1) })).collect::<Vec<_>>(),
        "kept": kept,
    });
    if b(a, "dry_run").unwrap_or(false) {
        return Ok(json!({ "valid": true, "matches": preview }));
    }
    let shared = rule.shared;
    let mem = srv.store().apply_with(MemoryOp::AddRule { rule: rule.clone() }, Some(total))?;
    srv.backend().memory_changed();
    let saved = mem.rules.iter().find(|r| r.pattern == rule.pattern && r.source == Source::Agent).cloned().ok_or("Rule wasn't saved.")?;
    let mut out = obj(json!({
        "rule": { "id": saved.id, "finding_rule": disko_memory::rules::finding_rule(&saved), "pattern": saved.pattern, "safety": kebab(saved.safety) },
        "matches": preview,
        "next": format!("Its findings appear in clutter (query: \"learned.{}\"); clean(ids) moves them to the Trash. Routines don't take agent rules; if the user wants it cleaned regularly, they can make it their own rule in Disko.", saved.id),
    }));
    if asked_safe {
        out.insert("note".into(), json!("Saved as review: rules from agents never delete permanently. The user can mark it safe in Disko."));
    }
    if shared {
        let ob = srv.store().outbox();
        let generalized = disko_memory::share::generalize(&saved.pattern, &home, &home.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        out.insert("shared".into(), json!({ "queued_as": generalized, "queued_total": ob.len(), "note": "Anonymized; sent when a community endpoint is configured." }));
    }
    Ok(Value::Object(out))
}

fn set_routine(srv: &Server, a: &Value, t: i64) -> Result<Value, String> {
    let id = s(a, "id").map(str::to_string);
    let existing = id.as_ref().and_then(|id| srv.store().memory().routines.into_iter().find(|r| &r.id == id));
    let mut r = existing.clone().unwrap_or(Routine {
        id: id.clone().unwrap_or_default(),
        title: String::new(),
        select: RoutineSelect::default(),
        every_days: 2,
        min_age_days: 0,
        enabled: false,
        review: false,
        last_run: None,
        last_freed: None,
    });
    if let Some(x) = s(a, "title") {
        r.title = x.to_string();
    }
    if a.get("categories").is_some() {
        r.select.categories = strs(a, "categories").iter().map(|c| category(c)).collect::<Result<_, _>>()?;
    }
    if a.get("rules").is_some() {
        r.select.rules = strs(a, "rules");
    }
    if let Some(x) = a.get("every_days").and_then(Value::as_u64) {
        r.every_days = x.min(10_000) as u32;
    }
    if let Some(x) = a.get("min_age_days").and_then(Value::as_u64) {
        r.min_age_days = x.min(10_000) as u32;
    }
    if let Some(x) = b(a, "enabled") {
        r.enabled = x;
    }
    if let Some(x) = b(a, "review") {
        r.review = x;
    }
    if existing.is_none() && r.title.is_empty() {
        return Err("A new routine needs a `title`.".into());
    }
    let mem = srv.store().apply(MemoryOp::SetRoutine { routine: r.clone() })?;
    srv.backend().memory_changed();
    let saved = mem.routines.iter().find(|x| (existing.is_some() && x.id == r.id) || x.title == r.title).cloned().ok_or("Routine wasn't saved.")?;
    let findings = srv.backend().clutter().unwrap_or_default();
    let keep = srv.store().keep().clone();
    let would: Vec<Finding> = disko_memory::select(&findings, &saved, t, &keep);
    Ok(json!({
        "routine": routine_row(&saved, t),
        "would_take_now": { "count": would.len(), "size": fmt_bytes(would.iter().map(|f| f.bytes).sum()) },
        "next": "run_routine(id) to run it now; Disko also runs enabled routines when they're due.",
    }))
}

fn run_routine(srv: &Server, a: &Value, t: i64) -> Result<Value, String> {
    let dry = b(a, "dry_run").unwrap_or(false);
    let mem = srv.store().memory();
    let routines: Vec<Routine> = match s(a, "id") {
        Some(id) => vec![mem.routines.iter().find(|r| r.id == id).cloned().ok_or_else(|| format!("No routine `{}`.", id))?],
        None => mem.routines.iter().filter(|r| r.due(t)).cloned().collect(),
    };
    if routines.is_empty() {
        return Ok(json!({ "ran": [], "note": "No routine is due." }));
    }
    let findings = srv.backend().clutter()?;
    let keep = srv.store().keep().clone();
    let mut out = vec![];
    for r in routines {
        let mut reasons: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        let mut chosen: Vec<&Finding> = vec![];
        for f in &findings {
            match reject_reason(f, &r, t, &keep) {
                None => chosen.push(f),
                Some(why) if why != "not selected by this routine" => {
                    let e = reasons.entry(why).or_default();
                    e.0 += 1;
                    e.1 += f.bytes;
                }
                _ => {}
            }
        }
        let skipped: Vec<Value> = reasons.iter().map(|(w, (c, b))| json!({ "why": w, "count": c, "size": fmt_bytes(*b) })).collect();
        let planned: u64 = chosen.iter().map(|f| f.bytes).sum();
        if dry {
            out.push(json!({ "id": r.id, "would_clean": chosen.len(), "size": fmt_bytes(planned),
                "items": chosen.iter().take(20).map(|f| json!({ "id": f.id, "title": f.title, "size": fmt_bytes(f.bytes) })).collect::<Vec<_>>(),
                "skipped": skipped }));
            continue;
        }
        // Started by an agent: to the Trash (or the tool's own cleanup), never deleted permanently.
        let req = trash_request(&chosen);
        let res = if chosen.is_empty() { Ok(Default::default()) } else { srv.backend().clean(&req) };
        match res {
            Ok(res) => {
                let freed = res.freed;
                srv.store().record_run(&r.id, t, freed)?;
                let mut m = obj(json!({ "id": r.id, "cleaned": res.removed.len(), "freed": fmt_bytes(freed), "freed_bytes": freed, "skipped": skipped }));
                if !res.failed.is_empty() {
                    m.insert("failed".into(), json!(res.failed.iter().map(|f| json!({ "path": f.path, "error": f.error })).collect::<Vec<_>>()));
                }
                out.push(Value::Object(m));
            }
            Err(e) => out.push(json!({ "id": r.id, "error": e })),
        }
    }
    if !dry {
        srv.backend().memory_changed();
    }
    Ok(json!({ "ran": out, "dry_run": dry }))
}
