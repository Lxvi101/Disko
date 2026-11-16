//! End-to-end JSON-RPC tests against a fake backend (no processes, no real disk changes).

use super::*;
use disko_core::{Action, Category, CleanFailure, Finding, HiddenSpace, Safety, Source, Trashed};
use disko_memory::MemoryOp;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const DAY: i64 = 86_400;

struct Fake {
    findings: Mutex<Vec<Finding>>,
    cleaned: Mutex<Vec<CleanRequest>>,
    changed: AtomicUsize,
}

fn finding(id: &str, cat: Category, safety: Safety, bytes: u64, age_days: i64, path: &str) -> Finding {
    Finding {
        id: id.into(),
        rule: format!("{}.fake", serde_json::to_value(cat).unwrap().as_str().unwrap()),
        category: cat,
        title: id.into(),
        detail: None,
        icon: "box".into(),
        paths: vec![path.into()],
        bytes,
        last_used: Some(disko_memory::now() - age_days * DAY),
        safety,
        action: if safety == Safety::Safe { Action::Delete } else { Action::Trash },
        regrows: safety == Safety::Safe,
        why: "fake".into(),
        busy: None,
        admin: false,
        source: Source::Builtin,
    }
}

fn node(path: &str, bytes: u64) -> TreeNode {
    TreeNode {
        name: path.rsplit('/').next().unwrap().into(),
        path: path.into(),
        kind: "dir".into(),
        bytes,
        files: 3,
        newest: Some(disko_memory::now() - 3 * DAY),
        clutter: None,
        error: None,
        children: None,
    }
}

impl Backend for Fake {
    fn volume(&self) -> Result<VolumeInfo, String> {
        Ok(VolumeInfo {
            name: "Macintosh HD".into(),
            total: 500_000_000_000,
            used: 400_000_000_000,
            free: 100_000_000_000,
            purgeable: 0,
            scanned: 350_000_000_000,
            hidden: vec![HiddenSpace { kind: "snapshots".into(), label: "Local snapshots".into(), bytes: 20_000_000_000, why: "Time Machine".into() }],
        })
    }
    fn freshness(&self) -> Freshness {
        Freshness { scanning: false, phase: "done".into(), fraction: 1.0, age_s: Some(30), files: 1000 }
    }
    fn tree(&self, path: Option<&str>, _depth: u32) -> Result<TreeNode, String> {
        let mut n = node(path.unwrap_or("/"), 300);
        n.children = Some((0..20).map(|i| node(&format!("{}/c{}", path.unwrap_or(""), i), 20 - i)).collect());
        Ok(n)
    }
    fn largest(&self, _under: Option<&str>, limit: usize, _files_only: bool) -> Result<Vec<TreeNode>, String> {
        Ok((0..limit.min(3)).map(|i| node(&format!("/big/{}", i), 1_000_000_000 - i as u64)).collect())
    }
    fn search(&self, query: &str, _limit: usize) -> Result<Vec<TreeNode>, String> {
        Ok(vec![node(&format!("/Users/x/Dev/a/{}", query), 5_000_000), node(&format!("/Users/x/Dev/b/{}", query), 7_000_000)])
    }
    fn clutter(&self) -> Result<Vec<Finding>, String> {
        Ok(self.findings.lock().unwrap().clone())
    }
    fn explain(&self, path: &str) -> Result<Value, String> {
        Ok(json!({ "bytes": 1234, "category": "build", "rule": "build.fake", "regrows": true, "path_seen": path }))
    }
    fn clean(&self, req: &CleanRequest) -> Result<CleanResult, String> {
        self.cleaned.lock().unwrap().push(req.clone());
        let mut f = self.findings.lock().unwrap();
        let mut r = CleanResult::default();
        for id in req.ids.iter().chain(&req.trash_ids) {
            if let Some(i) = f.iter().position(|x| &x.id == id) {
                let x = f.remove(i);
                r.freed += x.bytes;
                r.removed.push(x.id.clone());
                if req.trash_ids.contains(id) || x.action == Action::Trash {
                    r.trashed.push(Trashed { from: x.paths[0].clone(), to: format!("/Users/x/.Trash/{}", x.id), bytes: x.bytes });
                }
            }
        }
        for p in &req.paths {
            r.trashed.push(Trashed { from: p.clone(), to: "/Users/x/.Trash/p".into(), bytes: 10 });
            r.freed += 10;
        }
        if req.ids.iter().any(|i| i == "fails") {
            r.failed.push(CleanFailure { path: "/x".into(), error: "nope".into() });
        }
        Ok(r)
    }
    fn apps(&self) -> Result<Value, String> {
        Ok(json!([
            { "name": "Small", "bundle_id": "com.s", "path": "/Applications/Small.app", "app_bytes": 10, "data_bytes": 5, "data_paths": ["/a"], "running": false, "apple": false },
            { "name": "Big", "bundle_id": "com.b", "path": "/Applications/Big.app", "app_bytes": 1_000_000_000, "data_bytes": 2_000_000_000, "data_paths": [], "running": true, "apple": false, "last_used": disko_memory::now() - 40 * DAY }
        ]))
    }
    fn sessions(&self) -> Result<Value, String> {
        Ok(json!([{ "tool": "claude-code", "name": "Claude Code", "icon": "claude", "bytes": 300, "projects": [
            { "name": "disko", "path": "/Users/x/Dev/disko", "bytes": 200, "sessions": [{ "id": "s1", "bytes": 150, "paths": ["/p1"] }, { "id": "s2", "bytes": 50, "paths": [] }] },
            { "name": "other", "bytes": 100, "sessions": [] }
        ]}]))
    }
    fn rescan(&self) -> Result<(), String> {
        Ok(())
    }
    fn memory_changed(&self) {
        self.changed.fetch_add(1, Ordering::SeqCst);
    }
}

fn tempdir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("disko-mcp-test-{}-{}-{}", tag, std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Env {
    srv: Server,
    fake: Arc<Fake>,
    dir: PathBuf,
    next: std::cell::Cell<u64>,
}

impl Env {
    fn new(tag: &str) -> Env {
        let dir = tempdir(tag);
        let d = dir.to_string_lossy().to_string();
        let fake = Arc::new(Fake {
            findings: Mutex::new(vec![
                finding("old-target", Category::Build, Safety::Safe, 3_000_000_000, 5, &format!("{}/proj/target", d)),
                finding("fresh-target", Category::Build, Safety::Safe, 2_000_000_000, 0, &format!("{}/live/target", d)),
                finding("kept-nm", Category::Build, Safety::Safe, 900_000_000, 9, &format!("{}/keepme/node_modules", d)),
                finding("old-video", Category::Forgotten, Safety::Review, 4_000_000_000, 400, &format!("{}/Movies/x.mov", d)),
                finding("tiny-log", Category::Logs, Safety::Safe, 1000, 9, &format!("{}/x.log", d)),
                Finding { busy: Some("Xcode is running".into()), ..finding("busy-dd", Category::Developer, Safety::Safe, 5_000_000_000, 3, &format!("{}/DerivedData", d)) },
                Finding { admin: true, ..finding("root-cache", Category::Cache, Safety::Safe, 100_000_000, 3, "/Library/Caches/com.fake") },
                Finding {
                    action: Action::Command { argv: vec!["brew".into(), "cleanup".into()], label: "brew cleanup".into() },
                    ..finding("brew", Category::Packages, Safety::Safe, 700_000_000, 3, &format!("{}/brew-cache", d))
                },
            ]),
            cleaned: Mutex::new(vec![]),
            changed: AtomicUsize::new(0),
        });
        // `~` is the test dir, so learned rules (home only) can point at fixtures.
        let store = disko_memory::Store::open(dir.join("support"), dir.clone());
        let srv = Server::new(fake.clone(), store);
        Env { srv, fake, dir, next: std::cell::Cell::new(1) }
    }

    fn rpc(&self, method: &str, params: Value) -> Value {
        let id = self.next.get();
        self.next.set(id + 1);
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let out = self.srv.handle_line(&line).expect("reply");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"], id);
        v
    }

    /// Call a tool; returns (parsed result, is_error).
    fn call(&self, name: &str, args: Value) -> (Value, bool) {
        let v = self.rpc("tools/call", json!({ "name": name, "arguments": args }));
        let r = &v["result"];
        let text = r["content"][0]["text"].as_str().unwrap().to_string();
        let err = r["isError"].as_bool().unwrap();
        (serde_json::from_str(&text).unwrap_or(Value::String(text)), err)
    }

    fn ok(&self, name: &str, args: Value) -> Value {
        let (v, err) = self.call(name, args);
        assert!(!err, "{} failed: {}", name, v);
        v
    }

    fn p(&self, rel: &str) -> String {
        format!("{}/{}", self.dir.display(), rel)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn protocol_basics() {
    let e = Env::new("proto");
    let init = e.rpc("initialize", json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t" } }));
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "disko");
    assert!(init["result"]["instructions"].as_str().unwrap().contains("keep list always wins"));
    let init = e.rpc("initialize", json!({ "protocolVersion": "1999-01-01" }));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(e.srv.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    assert!(e.srv.handle_line("   ").is_none());
    assert_eq!(e.rpc("ping", json!({}))["result"], json!({}));
    assert_eq!(e.rpc("nope/method", json!({}))["error"]["code"], -32601);
    let bad: Value = serde_json::from_str(&e.srv.handle_line("{not json").unwrap()).unwrap();
    assert_eq!(bad["error"]["code"], -32700);

    let tools = e.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in ["overview", "largest", "tree", "search", "clutter", "explain", "clean", "apps", "sessions", "rescan", "memory", "keep", "unkeep", "add_rule", "remove_rule", "set_routine", "remove_routine", "run_routine"] {
        assert!(names.contains(&want), "missing {}", want);
    }
    for t in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(t["description"].as_str().unwrap().len() > 20);
    }
    let (msg, err) = e.call("no_such_tool", json!({}));
    assert!(err && msg.as_str().unwrap().contains("Unknown tool"));

    // Batch.
    let out = e.srv.handle(&json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }, { "jsonrpc": "2.0", "method": "notifications/x" }])).unwrap();
    assert_eq!(out.as_array().unwrap().len(), 1);
}

#[test]
fn serve_over_lines() {
    let e = Env::new("serve");
    let input = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }).to_string(),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "overview", "arguments": {} } }).to_string(),
    ]
    .join("\n");
    let mut out = vec![];
    e.srv.serve(std::io::Cursor::new(input), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["id"], 2);
}

#[test]
fn read_tools_are_compact() {
    let e = Env::new("read");
    let o = e.ok("overview", json!({}));
    assert_eq!(o["disk"]["free"], "100 GB");
    assert_eq!(o["hidden"][0]["size"], "20 GB");
    assert_eq!(o["memory"]["routines"][0]["id"], "build-artifacts");
    assert_eq!(o["memory"]["routines"][0]["enabled"], false);
    assert!(o["clutter"]["by_category"].as_array().unwrap().len() >= 4);

    let l = e.ok("largest", json!({ "limit": 2 }));
    assert_eq!(l["items"].as_array().unwrap().len(), 2);
    assert_eq!(l["items"][0]["size"], "1.0 GB");
    assert_eq!(l["items"][0]["idle_days"], 3);

    let t = e.ok("tree", json!({ "path": "/x", "limit": 5 }));
    assert_eq!(t["children"].as_array().unwrap().len(), 5);
    assert_eq!(t["more"]["count"], 15);
    let (_, err) = e.call("tree", json!({ "path": "relative" }));
    assert!(err);

    let s = e.ok("search", json!({ "query": ".turbo" }));
    assert_eq!(s["count"], 2);
    let (_, err) = e.call("search", json!({}));
    assert!(err);

    let a = e.ok("apps", json!({}));
    assert_eq!(a["apps"][0]["name"], "Big");
    assert_eq!(a["apps"][0]["size"], "3.0 GB");
    assert_eq!(a["apps"][0]["idle_days"], 40);
    assert!(a["apps"][0].get("data_paths").is_none());
    assert_eq!(e.ok("apps", json!({ "query": "com.s", "details": true }))["apps"][0]["data_paths"][0], "/a");

    let s = e.ok("sessions", json!({}));
    assert_eq!(s["tools"][0]["projects"][0]["sessions"], 2);
    assert!(s["tools"][0]["projects"][0].get("session_list").is_none());
    let s = e.ok("sessions", json!({ "tool": "claude-code" }));
    assert_eq!(s["tools"][0]["projects"][0]["session_list"][0]["id"], "s1");
    assert!(e.ok("sessions", json!({ "tool": "codex" }))["tools"].as_array().unwrap().is_empty());
    assert_eq!(e.ok("rescan", json!({}))["started"], true);
}

#[test]
fn clutter_filters_and_keep_hides() {
    let e = Env::new("clutter");
    let c = e.ok("clutter", json!({}));
    assert_eq!(c["count"], 7, "tiny-log is below 1 MB by default: {}", c);
    assert_eq!(c["findings"][0]["id"], "busy-dd");
    assert_eq!(c["findings"][0]["busy"], "Xcode is running");
    assert_eq!(e.ok("clutter", json!({ "min_bytes": 0 }))["count"], 8);
    let c = e.ok("clutter", json!({ "category": "build", "safety": "safe" }));
    assert_eq!(c["count"], 3);
    assert_eq!(e.ok("clutter", json!({ "category": ["build", "forgotten"], "limit": 1 }))["shown"], 1);
    assert_eq!(e.ok("clutter", json!({ "query": "keepme" }))["count"], 1);
    let (msg, err) = e.call("clutter", json!({ "category": "junk" }));
    assert!(err && msg.as_str().unwrap().contains("Unknown category"));

    let k = e.ok("keep", json!({ "pattern": e.p("keepme"), "note": "offline work" }));
    assert_eq!(k["now_protects"]["findings"], 1);
    let c = e.ok("clutter", json!({ "category": "build" }));
    assert_eq!(c["count"], 2);
    assert_eq!(c["kept_hidden"]["count"], 1);
    let c = e.ok("clutter", json!({ "category": "build", "include_kept": true }));
    assert_eq!(c["count"], 3);
    assert!(c["findings"].as_array().unwrap().iter().any(|f| f.get("kept").is_some()));
    assert!(e.fake.changed.load(Ordering::SeqCst) >= 1);

    let m = e.ok("memory", json!({}));
    let id = m["keep"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(m["keep"][0]["source"], "agent");
    e.ok("unkeep", json!({ "id": id }));
    assert_eq!(e.ok("clutter", json!({ "category": "build" }))["count"], 3);
}

#[test]
fn clean_respects_safety_keep_and_busy() {
    let e = Env::new("clean");
    std::fs::create_dir_all(e.p("loose")).unwrap();
    e.ok("keep", json!({ "pattern": e.p("keepme") }));

    // Dry run touches nothing.
    let d = e.ok("clean", json!({ "ids": ["old-target", "kept-nm"], "dry_run": true }));
    assert_eq!(d["would_clean"].as_array().unwrap().len(), 1);
    assert_eq!(d["skipped"][0]["item"], "kept-nm");
    assert!(e.fake.cleaned.lock().unwrap().is_empty());

    let r = e.ok(
        "clean",
        json!({ "ids": ["old-target", "kept-nm", "busy-dd", "root-cache", "old-video", "brew", "ghost"],
                "paths": [e.p("loose"), e.p("missing"), "/System/Library", "~", e.p("keepme/node_modules/x"), e.p("live/target")] }),
    );
    let req = e.fake.cleaned.lock().unwrap()[0].clone();
    // Nothing is deleted permanently through MCP: only the tool command keeps its own action.
    assert_eq!(req.ids, vec!["brew"]);
    assert_eq!(req.trash_ids, vec!["old-target", "old-video", "fresh-target"], "a path equal to a finding is cleaned as that finding");
    assert_eq!(req.paths, vec![e.p("loose")]);
    let skipped: Vec<String> = r["skipped"].as_array().unwrap().iter().map(|s| s["item"].as_str().unwrap().to_string()).collect();
    for s in ["kept-nm", "busy-dd", "root-cache", "ghost", "/System/Library"] {
        assert!(skipped.iter().any(|x| x == s), "{} should be skipped: {:?}", s, skipped);
    }
    assert!(skipped.contains(&e.p("missing")) && skipped.contains(&e.p("keepme/node_modules/x")));
    assert!(r["freed_bytes"].as_u64().unwrap() > 9_000_000_000);
    assert!(r["undo"].is_string());

    // `trash: false` changes nothing: still the Trash.
    e.fake.findings.lock().unwrap().push(finding("t1", Category::Build, Safety::Safe, 5, 5, "/tmp/disko-fake-t1"));
    e.ok("clean", json!({ "ids": ["t1"], "trash": false }));
    let req = e.fake.cleaned.lock().unwrap()[1].clone();
    assert_eq!(req.trash_ids, vec!["t1"]);
    assert!(req.ids.is_empty());

    // A finding claiming Delete (whatever its safety) goes to the Trash too.
    let mut bad = finding("bad", Category::Leftovers, Safety::Review, 5, 5, "/tmp/disko-fake-bad");
    bad.action = Action::Delete;
    e.fake.findings.lock().unwrap().push(bad);
    e.ok("clean", json!({ "ids": ["bad"] }));
    let req = e.fake.cleaned.lock().unwrap()[2].clone();
    assert_eq!((req.ids.len(), req.trash_ids.clone()), (0, vec!["bad".to_string()]));
    let (_, err) = e.call("clean", json!({}));
    assert!(err);

    // `clutter` tells agents what clean will do.
    let c = e.ok("clutter", json!({ "min_bytes": 0, "limit": 500 }));
    for f in c["findings"].as_array().unwrap() {
        let how = f["action"].as_str().unwrap();
        assert!(how == "trash" || how.starts_with("command:"), "{}", f);
    }
}

#[test]
fn rules_explain_and_sharing() {
    let e = Env::new("rules");
    std::fs::create_dir_all(e.p("ws/a/.turbo")).unwrap();
    std::fs::write(e.p("ws/a/.turbo/blob"), vec![1u8; 20_000]).unwrap();
    std::fs::create_dir_all(e.p("ws/b/.turbo")).unwrap();

    let pattern = "~/ws/*/.turbo";
    let d = e.ok("add_rule", json!({ "pattern": pattern, "title": "Turbo cache", "category": "build", "safety": "safe", "regrows": true, "why": "Turborepo cache; rebuilt on the next build.", "dry_run": true }));
    assert_eq!(d["matches"]["count"], 2);
    assert!(d["matches"]["bytes"].as_u64().unwrap() >= 20_000);
    assert!(e.ok("memory", json!({}))["rules"].as_array().unwrap().is_empty());

    let r = e.ok("add_rule", json!({ "pattern": pattern, "title": "Turbo cache", "category": "build", "safety": "safe", "regrows": true, "why": "Turborepo cache", "share": true }));
    assert_eq!(r["rule"]["id"], "turbo-cache");
    assert_eq!(r["rule"]["finding_rule"], "learned.turbo-cache");
    assert_eq!(r["shared"]["queued_total"], 1);
    let queued = r["shared"]["queued_as"].as_str().unwrap();
    assert!(!queued.contains("disko-mcp-test"), "anonymized: {}", queued);

    // ** rules preview through the index (search).
    let r = e.ok("add_rule", json!({ "pattern": "~/**/.turbo", "title": "Turbo anywhere", "category": "build", "safety": "safe", "dry_run": true }));
    assert_eq!(r["matches"]["count"], 0, "fake search paths are under /Users/x, not this HOME");

    for pattern in ["~/*", "~/Documents", "~/Dev/app/.git", "~/Dev/*/.GIT/objects", "~/Library/Mail/V10", "~/Library/Application Support/x",
                    "~/.ssh/id_rsa", "~/.config/gh", "~/**/Thesis", "/tmp/x/build", "/Library/Caches/com.x"] {
        let (msg, err) = e.call("add_rule", json!({ "pattern": pattern, "title": "x", "category": "build", "safety": "safe" }));
        assert!(err, "should fail: {} ({})", pattern, msg);
    }

    e.ok("keep", json!({ "pattern": e.p("ws/a") }));
    let x = e.ok("explain", json!({ "path": e.p("ws/a/.turbo") }));
    assert_eq!(x["size"], "1.2 KB");
    assert_eq!(x["learned_rule"]["id"], "turbo-cache");
    assert_eq!(x["kept"]["pattern"], e.p("ws/a"));
    let x = e.ok("explain", json!({ "path": "/System/Library" }));
    assert!(x["protected"].is_string());

    e.ok("remove_rule", json!({ "id": "turbo-cache" }));
    let m = e.ok("memory", json!({}));
    assert!(m["rules"].as_array().unwrap().is_empty());
    assert!(m["community"]["queued_submissions"].as_array().unwrap().is_empty());
    assert!(m["community"]["rules"].as_u64().unwrap() > 0);
}

#[test]
fn agents_never_delete_permanently() {
    let e = Env::new("no-delete");
    std::fs::create_dir_all(e.p("Documents/Thesis")).unwrap();
    std::fs::write(e.p("Documents/Thesis/ch1.tex"), b"years of work").unwrap();

    // The review's attack: a "safe cache" rule over personal files. It's saved as review.
    let r = e.ok("add_rule", json!({ "pattern": "~/Documents/Thesis", "title": "Cache", "category": "cache", "safety": "safe", "regrows": true }));
    assert_eq!(r["rule"]["safety"], "review");
    assert!(r["note"].as_str().unwrap().contains("review"));
    let m = e.ok("memory", json!({}));
    assert_eq!(m["rules"][0]["safety"], "review");
    assert_eq!(m["rules"][0]["source"], "agent");

    // Even if a finding from it claimed safe + Delete, clean by id or by path only trashes it.
    let mut f = finding("thesis", Category::Cache, Safety::Safe, 50_000_000, 30, &e.p("Documents/Thesis"));
    f.rule = "learned.cache".into();
    f.source = Source::Agent;
    e.fake.findings.lock().unwrap().push(f);
    let c = e.ok("clutter", json!({ "query": "thesis" }));
    assert_eq!(c["findings"][0]["action"], "trash");
    e.ok("clean", json!({ "ids": ["thesis"] }));
    e.fake.findings.lock().unwrap().push({
        let mut f = finding("thesis", Category::Cache, Safety::Safe, 50_000_000, 30, &e.p("Documents/Thesis"));
        f.rule = "learned.cache".into();
        f.source = Source::Agent;
        f
    });
    e.ok("clean", json!({ "paths": [e.p("Documents/Thesis")] }));
    for req in e.fake.cleaned.lock().unwrap().iter() {
        assert!(req.ids.is_empty(), "nothing with its own (Delete) action: {:?}", req);
        assert_eq!(req.trash_ids, vec!["thesis"]);
    }

    // Routines never take agent or community rule findings, even ones selected by category.
    e.fake.findings.lock().unwrap().push({
        let mut f = finding("agent-build", Category::Build, Safety::Safe, 1_000_000_000, 30, &e.p("x/build"));
        f.rule = "learned.x".into();
        f.source = Source::Agent;
        f
    });
    e.fake.findings.lock().unwrap().push({
        let mut f = finding("community-build", Category::Build, Safety::Safe, 1_000_000_000, 30, &e.p("y/.dart_tool"));
        f.rule = "community.dart-tool".into();
        f.source = Source::Community;
        f
    });
    e.ok("set_routine", json!({ "id": "build-artifacts", "enabled": true, "review": true }));
    let d = e.ok("run_routine", json!({ "id": "build-artifacts", "dry_run": true }));
    let ids: Vec<&str> = d["ran"][0]["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert!(!ids.contains(&"agent-build") && !ids.contains(&"community-build"), "{:?}", ids);
}

#[test]
fn agents_cant_undo_the_users_choices() {
    let e = Env::new("user-wins");
    // The user's keep entry: an agent can't remove it.
    e.srv.store().apply(MemoryOp::Keep { pattern: e.p("Thesis"), note: None, source: None }).unwrap();
    let (msg, err) = e.call("unkeep", json!({ "id": e.p("Thesis") }));
    assert!(err && msg.as_str().unwrap().contains("user"), "{}", msg);
    let m = e.ok("memory", json!({}));
    assert_eq!(m["keep"].as_array().unwrap().len(), 1);
    let id = m["keep"][0]["id"].as_str().unwrap().to_string();
    let (_, err) = e.call("unkeep", json!({ "id": id }));
    assert!(err);
    // Its own: fine.
    e.ok("keep", json!({ "pattern": e.p("agent-kept") }));
    e.ok("unkeep", json!({ "id": e.p("agent-kept") }));

    // The user's rule: an agent can't overwrite it by id or shadow it by pattern.
    let mine = disko_memory::LearnedRule {
        id: "turbo".into(),
        pattern: "~/.turbo/cache".into(),
        category: Category::Build,
        title: "Turbo".into(),
        why: String::new(),
        regrows: true,
        safety: Safety::Safe,
        source: Source::User,
        added: 0,
        shared: false,
    };
    e.srv.store().apply(MemoryOp::AddRule { rule: mine }).unwrap();
    let (msg, err) = e.call("add_rule", json!({ "id": "turbo", "pattern": "~/Documents/x", "title": "x", "category": "cache" }));
    assert!(err && msg.as_str().unwrap().contains("user"), "{}", msg);
    let (_, err) = e.call("add_rule", json!({ "pattern": "~/.Turbo/cache", "title": "Shadow", "category": "cache" }));
    assert!(err);
    let m = e.ok("memory", json!({}));
    assert_eq!(m["rules"].as_array().unwrap().len(), 1);
    assert_eq!(m["rules"][0]["safety"], "safe");
}

#[test]
fn routines_end_to_end() {
    let e = Env::new("routines");
    // Disabled by default: nothing due.
    assert_eq!(e.ok("run_routine", json!({}))["note"], "No routine is due.");

    let s = e.ok("set_routine", json!({ "id": "build-artifacts", "enabled": true }));
    assert_eq!(s["routine"]["enabled"], true);
    assert_eq!(s["routine"]["every_days"], 2);
    assert_eq!(s["would_take_now"]["count"], 2, "old-target + kept-nm (no keep yet); fresh one is too new");
    e.ok("keep", json!({ "pattern": e.p("keepme") }));

    let d = e.ok("run_routine", json!({ "id": "build-artifacts", "dry_run": true }));
    assert_eq!(d["ran"][0]["would_clean"], 1);
    let reasons: Vec<&str> = d["ran"][0]["skipped"].as_array().unwrap().iter().map(|s| s["why"].as_str().unwrap()).collect();
    assert!(reasons.iter().any(|r| r.contains("keep list")) && reasons.iter().any(|r| r.contains("used in the last")));
    assert!(e.fake.cleaned.lock().unwrap().is_empty());

    let r = e.ok("run_routine", json!({}));
    assert_eq!(r["ran"][0]["id"], "build-artifacts");
    assert_eq!(r["ran"][0]["freed"], "3.0 GB");
    let req = e.fake.cleaned.lock().unwrap()[0].clone();
    assert_eq!(req.trash_ids, vec!["old-target"], "run from MCP, routine items go to the Trash");
    assert!(req.ids.is_empty());
    let m = e.ok("memory", json!({}));
    assert_eq!(m["routines"][0]["last_freed"], "3.0 GB");
    assert_eq!(m["routines"][0]["due"], false);
    assert_eq!(e.ok("run_routine", json!({}))["note"], "No routine is due.");

    // A new routine needs a title and a selector; review items only when asked.
    let (_, err) = e.call("set_routine", json!({ "categories": ["logs"] }));
    assert!(err);
    let (_, err) = e.call("set_routine", json!({ "title": "Nothing" }));
    assert!(err);
    let s = e.ok("set_routine", json!({ "title": "Old videos", "categories": ["forgotten"], "every_days": 30, "min_age_days": 365, "enabled": true }));
    assert_eq!(s["would_take_now"]["count"], 0);
    let s = e.ok("set_routine", json!({ "id": "old-videos", "review": true }));
    assert_eq!(s["would_take_now"]["count"], 1);
    e.ok("remove_routine", json!({ "id": "old-videos" }));
    let (_, err) = e.call("run_routine", json!({ "id": "old-videos" }));
    assert!(err);
}
