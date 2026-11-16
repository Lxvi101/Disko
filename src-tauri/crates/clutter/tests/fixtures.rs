//! Fixture tests: everything happens in temp folders. Nothing here touches real user data
//! (the one Trash round-trip moves our own temp file and puts it back).

use disko_clutter::apps::{App, Apps};
use disko_clutter::procs::{Proc, Procs};
use disko_clutter::{probes, walk, Clutter, Env, PatternRule};
use disko_core::{finding_id, Action, Category, Finding, Safety, Source};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct Fx {
    root: PathBuf,
    home: PathBuf,
    sys: PathBuf,
}

impl Fx {
    fn new(name: &str) -> Fx {
        let root = std::env::temp_dir().join(format!("disko-clutter-{}-{}-{}", name, std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let home = root.join("home");
        let sys = root.join("sys");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&sys).unwrap();
        Fx { root, home, sys }
    }
    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        let p = self.home.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![7u8; bytes]).unwrap();
        p
    }
    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.home.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn clutter(&self, procs: Vec<Proc>, apps: Vec<App>, rules: Vec<PatternRule>) -> Arc<Clutter> {
        Clutter::with_env(Env::fixture(self.home.clone(), self.sys.clone()), rules, Some(Procs { list: procs, complete: true }), Some(Apps::from_list(apps)))
    }
    fn walk(&self, c: &Clutter) -> HashMap<String, Finding> {
        let found = Mutex::new(HashMap::new());
        walk::walk(c, &self.home, &|f| {
            found.lock().unwrap().insert(f.id.clone(), f);
        });
        found.into_inner().unwrap()
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const MB: usize = 1024 * 1024;

fn old(path: &Path) {
    let st = std::process::Command::new("/usr/bin/touch").args(["-t", "202301010000"]).arg(path).status().unwrap();
    assert!(st.success());
}

fn by_rule<'a>(f: &'a HashMap<String, Finding>, rule: &str) -> Vec<&'a Finding> {
    f.values().filter(|x| x.rule == rule).collect()
}

fn app(path: &Path, id: &str, name: &str) -> App {
    App { path: path.to_path_buf(), id: id.to_ascii_lowercase(), name: name.into(), version: Some("1.0".into()) }
}

#[test]
fn build_output_needs_its_manifest_and_groups_per_project() {
    let fx = Fx::new("rules");
    // A monorepo: two node_modules and a Cargo target under one git root.
    fx.dir("dev/mono/.git");
    fx.file("dev/mono/package.json", 10);
    fx.file("dev/mono/node_modules/a/big.bin", 2 * MB);
    fx.file("dev/mono/web/package.json", 10);
    fx.file("dev/mono/web/node_modules/b/big.bin", 2 * MB);
    fx.file("dev/mono/web/.next/cache/x", MB);
    fx.file("dev/mono/core/Cargo.toml", 10);
    fx.file("dev/mono/core/target/debug/app", 2 * MB);
    fx.file("dev/mono/core/target/CACHEDIR.TAG", 10);
    // Same names without the proof next to them: somebody's data.
    fx.file("dev/photos/target/debug/pic.raw", 2 * MB);
    fx.file("dev/notes/node_modules/x/y.bin", 2 * MB);
    // Custom cargo target dir, found by its tag.
    fx.file("dev/tool/target-agents/CACHEDIR.TAG", 10);
    fx.file("dev/tool/target-agents/debug/x", 2 * MB);
    // Never inside tool stores or site-packages.
    fx.file(".nvm/versions/node/v1/lib/node_modules/npm/package.json", 10);
    fx.file(".nvm/versions/node/v1/lib/node_modules/npm/node_modules/z/big", 2 * MB);
    fx.file("dev/py/.venv/pyvenv.cfg", 10);
    fx.file("dev/py/.venv/lib/python3.12/site-packages/pkg/__pycache__/m.pyc", 2 * MB);
    fx.file("dev/py/pkg/__pycache__/m.pyc", 2 * MB);
    fx.file("dev/py/pyproject.toml", 10);

    let c = fx.clutter(vec![], vec![], vec![]);
    let f = fx.walk(&c);
    let build = by_rule(&f, "build");
    let mono = build.iter().find(|x| x.title == "mono").expect("monorepo grouped under its git root");
    assert_eq!(mono.paths.len(), 4, "{:?}", mono.paths);
    assert_eq!(mono.id, finding_id("build", &[disko_clutter::util::s(&fx.home.join("dev/mono"))]));
    assert_eq!(mono.safety, Safety::Safe);
    assert_eq!(mono.action, Action::Delete);
    assert!(mono.regrows);
    assert!(mono.detail.as_deref().unwrap().contains("node_modules"));
    assert!(build.iter().any(|x| x.title == "tool" && x.paths[0].ends_with("target-agents")), "{:#?}", build);
    assert!(!f.values().any(|x| x.paths.iter().any(|p| p.contains("/photos/") || p.contains("/notes/") || p.contains("/.nvm/") || p.contains("site-packages"))), "{:#?}", f.values().map(|x| &x.paths).collect::<Vec<_>>());
    // The venv is its own Review finding; the project's own __pycache__ is build output.
    let envs = by_rule(&f, "build.environments");
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].safety, Safety::Review);
    assert_eq!(envs[0].action, Action::Trash);
    assert!(build.iter().any(|x| x.title == "py" && x.paths.iter().any(|p| p.ends_with("pkg/__pycache__"))));

    // Stable ids across scans.
    c.reset();
    let again = fx.walk(&c);
    let mut a: Vec<&String> = f.keys().collect();
    let mut b: Vec<&String> = again.keys().collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

#[test]
fn app_caches_group_per_app_and_skip_uninstalled_apps() {
    let fx = Fx::new("appcache");
    let slack = fx.dir("Applications/Slack.app");
    fx.file("Library/Application Support/Slack/Local State", 10);
    fx.file("Library/Application Support/Slack/Cache/Cache_Data/a", 2 * MB);
    fx.file("Library/Application Support/Slack/Code Cache/js/b", 2 * MB);
    fx.file("Library/Application Support/Slack/GPUCache/c", MB);
    // A "Cache" folder without a Chromium profile next to it is left alone.
    fx.file("Library/Application Support/Notes/Cache/c", 2 * MB);
    // Sandboxed caches: installed app yes, Apple no.
    fx.file("Library/Containers/com.foo.bar/Data/Library/Caches/x", 2 * MB);
    fx.file("Library/Containers/com.apple.Safari/Data/Library/Caches/x", 2 * MB);
    let c = fx.clutter(vec![Proc { pid: 9, exe: format!("{}/Contents/MacOS/Slack", slack.display()), name: "Slack".into(), app: Some("Slack".into()), cwd: None }], vec![app(&slack, "com.tinyspeck.slackmacgap", "Slack"), app(&fx.home.join("Applications/Bar.app"), "com.foo.bar", "Bar")], vec![]);
    let f = fx.walk(&c);
    let chromium = by_rule(&f, "cache.chromium");
    assert_eq!(chromium.len(), 1, "{:#?}", chromium);
    assert_eq!(chromium[0].title, "Slack");
    assert_eq!(chromium[0].paths.len(), 3);
    assert_eq!(chromium[0].busy.as_deref(), Some("Slack is running"));
    let cont = by_rule(&f, "cache.container");
    assert_eq!(cont.len(), 1);
    assert_eq!(cont[0].title, "Bar");
}

#[test]
fn files_installers_and_forgotten() {
    let fx = Fx::new("files");
    let docker = fx.dir("Applications/Docker.app");
    fx.file("Downloads/Docker-4.32.0-arm64.dmg", 2 * MB);
    let big = fx.file("Movies/old-render.mov", 501 * MB);
    old(&big);
    let recent_big = fx.file("Movies/new-render.mov", 501 * MB);
    let c = fx.clutter(vec![], vec![app(&docker, "com.docker.docker", "Docker")], vec![]);
    let f = fx.walk(&c);
    let inst = by_rule(&f, "installers.file");
    assert_eq!(inst.len(), 1);
    assert_eq!(inst[0].title, "Docker");
    assert!(inst[0].why.contains("Docker is installed"));
    assert_eq!(inst[0].safety, Safety::Review);
    let forgotten = by_rule(&f, "forgotten.file");
    assert_eq!(forgotten.len(), 1);
    assert_eq!(forgotten[0].category, Category::Forgotten);
    assert!(forgotten[0].paths[0].ends_with("old-render.mov"));
    assert!(recent_big.exists());
    // The downloads probe finds the installer at scan start with the same id.
    let early = probes::system::downloads(&c);
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].id, inst[0].id);
}

#[test]
fn pattern_rules_match_in_walk_and_probe_with_one_id() {
    let fx = Fx::new("patterns");
    fx.file("stuff/a.cache/x", 2 * MB);
    fx.file("stuff/b.keep/x", 2 * MB);
    let rule = PatternRule { id: "user.stuff".into(), pattern: "~/stuff/*.cache".into(), category: Category::Cache, title: "Stuff cache".into(), why: "I said so".into(), regrows: true, safety: Safety::Safe, source: Source::User };
    let c = fx.clutter(vec![], vec![], vec![rule]);
    let f = fx.walk(&c);
    let hits = by_rule(&f, "user.stuff");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source, Source::User);
    let collected = Mutex::new(vec![]);
    c.probes(&|v| collected.lock().unwrap().extend(v));
    let probe_hits: Vec<Finding> = collected.into_inner().unwrap().into_iter().filter(|x| x.rule == "user.stuff").collect();
    assert_eq!(probe_hits.len(), 1);
    assert_eq!(probe_hits[0].id, hits[0].id);
}

#[test]
fn probes_find_versions_leftovers_and_trash() {
    let fx = Fx::new("probes");
    // Claude Code installed as versions with a launcher symlink.
    fx.file(".local/share/claude/versions/1.0.0", 2 * MB);
    let cur = fx.file(".local/share/claude/versions/2.0.0", 2 * MB);
    fx.file(".local/share/claude/versions/3.0.0-beta", 2 * MB);
    fx.dir(".local/bin");
    std::os::unix::fs::symlink(&cur, fx.home.join(".local/bin/claude")).unwrap();
    // nvm with default 22.
    fx.file(".nvm/versions/node/v20.1.0/bin/node", 2 * MB);
    fx.file(".nvm/versions/node/v22.3.0/bin/node", 2 * MB);
    fx.file(".nvm/alias/default", 0);
    fs::write(fx.home.join(".nvm/alias/default"), "22\n").unwrap();
    // VS Code obsolete extensions.
    fx.file(".vscode/extensions/ms.python-1.0.0/x", MB);
    fx.file(".vscode/extensions/ms.python-2.0.0/x", MB);
    fs::write(fx.home.join(".vscode/extensions/.obsolete"), r#"{"ms.python-1.0.0":true}"#).unwrap();
    // An uninstalled app's data, and an installed one's.
    let gone = [fx.file("Library/Application Support/com.gone.app/db", 2 * MB), fx.file("Library/Preferences/com.gone.app.plist", 100)];
    for g in &gone {
        old(g);
    }
    old(&fx.home.join("Library/Application Support/com.gone.app"));
    fx.file("Library/Application Support/com.here.app/db", 2 * MB);
    fx.file("Library/Preferences/com.here.app.plist", 100);
    // Empty Keystone agent and a dead one.
    fx.file("Library/LaunchAgents/com.google.keystone.agent.plist", 0);
    fs::write(fx.home.join("Library/LaunchAgents/com.dead.plist"), r#"<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>com.dead</string><key>Program</key><string>/nonexistent/bin/dead</string></dict></plist>"#).unwrap();
    // Trash.
    fx.file(".Trash/old.zip", MB);
    let c = fx.clutter(vec![], vec![app(&fx.home.join("Applications/Here.app"), "com.here.app", "Here")], vec![]);
    let collected = Mutex::new(vec![]);
    c.probes(&|v| collected.lock().unwrap().extend(v));
    let all = collected.into_inner().unwrap();
    let get = |rule: &str| all.iter().filter(|f| f.rule == rule).cloned().collect::<Vec<_>>();

    let rel = get("versions.releases");
    assert_eq!(rel.len(), 1, "{:#?}", rel);
    assert_eq!(rel[0].paths.len(), 1);
    assert!(rel[0].paths[0].ends_with("1.0.0"), "current and newer stay: {:?}", rel[0].paths);
    assert_eq!(rel[0].title, "Claude Code");

    let node = get("versions.node");
    assert_eq!(node.len(), 1);
    assert!(node[0].paths[0].ends_with("v20.1.0"));

    let ext = get("versions.extensions");
    assert_eq!(ext.len(), 1);
    assert!(ext[0].paths[0].ends_with("ms.python-1.0.0"));

    let left = get("leftovers.app");
    assert_eq!(left.len(), 1, "{:#?}", left);
    assert_eq!(left[0].paths.len(), 2);
    assert_eq!(left[0].safety, Safety::Review);
    assert!(left[0].why.contains("com.gone.app"));

    let agents = get("leftovers.launch-agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].paths.len(), 2);

    let trash = get("trash");
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].action, Action::Delete);
    assert_eq!(trash[0].safety, Safety::Review);
}

#[test]
fn retained_zip_is_verified_against_its_extracted_folder() {
    let fx = Fx::new("zip");
    let dir = fx.dir("Downloads/Tool");
    fs::write(dir.join("a.txt"), vec![1u8; 300_000]).unwrap();
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/b.bin"), vec![2u8; 300_000]).unwrap();
    let make_zip = |path: &Path, tweak: bool| {
        let f = fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("Tool/a.txt", opt).unwrap();
        std::io::Write::write_all(&mut z, &vec![1u8; 300_000]).unwrap();
        z.start_file("Tool/sub/b.bin", opt).unwrap();
        std::io::Write::write_all(&mut z, &vec![if tweak { 3u8 } else { 2u8 }; 300_000]).unwrap();
        z.finish().unwrap();
    };
    let zp = fx.home.join("Downloads/Tool.zip");
    make_zip(&zp, false);
    let budget = std::sync::atomic::AtomicU64::new(0);
    assert_eq!(probes::deep::verify_zip(&zp, &dir, &budget), Ok(2));
    let c = fx.clutter(vec![], vec![], vec![]);
    let found = probes::deep::archives(&c);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rule, "installers.extracted-zip");
    // One differing byte and it is not proposed.
    make_zip(&zp, true);
    assert!(probes::deep::verify_zip(&zp, &dir, &budget).is_err());
    assert!(probes::deep::archives(&c).is_empty());
}

fn fake(rule: &str, paths: Vec<PathBuf>, safety: Safety, action: Action) -> Finding {
    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
    Finding {
        id: finding_id(rule, &paths),
        rule: rule.into(),
        category: Category::Cache,
        title: rule.into(),
        detail: None,
        icon: "box".into(),
        paths,
        bytes: 0,
        last_used: None,
        safety,
        action,
        regrows: true,
        why: String::new(),
        busy: None,
        admin: false,
        source: Source::Builtin,
    }
}

#[test]
fn clean_deletes_safe_items_and_respects_keep_busy_protected_and_links() {
    let fx = Fx::new("clean");
    let a = fx.file("dev/a/node_modules/x/big", 2 * MB);
    let a_dir = a.parent().unwrap().parent().unwrap().to_path_buf();
    let kept = fx.file("dev/k/node_modules/x/big", MB);
    let kept_dir = kept.parent().unwrap().parent().unwrap().to_path_buf();
    let busy = fx.file("dev/b/node_modules/x/big", MB);
    let busy_dir = busy.parent().unwrap().parent().unwrap().to_path_buf();
    // A link to a folder: cleaning something reached through it is refused.
    let real = fx.file("real/cache/x", MB);
    std::os::unix::fs::symlink(fx.home.join("real"), fx.home.join("link")).unwrap();
    let via_link = fx.home.join("link/cache");

    let procs = vec![Proc { pid: 5, exe: "/usr/bin/node".into(), name: "node".into(), app: None, cwd: Some(busy_dir.clone()) }];
    let c = fx.clutter(procs, vec![], vec![]);
    let findings = vec![
        fake("t.a", vec![a_dir.clone()], Safety::Safe, Action::Delete),
        fake("t.k", vec![kept_dir.clone()], Safety::Safe, Action::Delete),
        fake("t.b", vec![busy_dir.clone()], Safety::Safe, Action::Delete),
        fake("t.l", vec![via_link.clone()], Safety::Safe, Action::Delete),
        fake("t.p", vec![PathBuf::from("/System/Library/Fonts")], Safety::Safe, Action::Delete),
    ];
    let kept_s = kept_dir.to_string_lossy().to_string();
    let mut steps = 0;
    let r = c.clean(&findings, &|p| p == kept_s, &mut |_, _, _| steps += 1);
    assert!(!a_dir.exists());
    assert!(r.freed >= 2 * MB as u64);
    assert!(r.removed.contains(&findings[0].id));
    assert!(kept_dir.exists() && r.skipped.iter().any(|s| s.path == kept_s));
    assert!(busy_dir.exists() && r.skipped.iter().any(|s| s.path == busy_dir.to_string_lossy() && s.error.contains("node")));
    assert!(real.exists() && r.failed.iter().any(|f| f.path == via_link.to_string_lossy()));
    assert!(Path::new("/System/Library/Fonts").exists() && r.failed.iter().any(|f| f.path == "/System/Library/Fonts"));
    assert!(steps >= findings.len());
}

#[test]
fn clean_runs_commands_once_and_review_goes_to_trash() {
    let fx = Fx::new("clean2");
    let d = fx.file("cache/tool/x", 2 * MB).parent().unwrap().to_path_buf();
    let argv = vec!["/bin/rm".to_string(), "-rf".into(), d.to_string_lossy().to_string()];
    let mut cmd = fake("t.cmd", vec![d.clone()], Safety::Safe, Action::Command { argv: argv.clone(), label: "rm".into() });
    cmd.bytes = 2 * MB as u64;
    let twin = Finding { id: "t.cmd2:1".into(), ..cmd.clone() };
    // Review + Delete is never deleted outright: it goes to the Trash.
    let rv = fx.file("old/forgotten.bin", MB);
    let review = fake("t.review", vec![rv.clone()], Safety::Review, Action::Delete);
    let c = fx.clutter(vec![], vec![], vec![]);
    let r = c.clean(&[cmd.clone(), twin.clone(), review.clone()], &|_| false, &mut |_, _, _| {});
    assert!(!d.exists());
    assert!(r.freed >= 2 * MB as u64, "{:?}", r);
    assert!(r.removed.contains(&cmd.id) && r.removed.contains(&twin.id));
    assert_eq!(r.trashed.len(), 1, "{:?}", r);
    let t = &r.trashed[0];
    assert_eq!(t.from, rv.to_string_lossy());
    // Moved to the Trash is not freed: only the command's 2 MB count.
    assert!(r.freed < 3 * MB as u64, "{:?}", r);
    assert!(t.bytes >= MB as u64);
    assert!(r.deleted.contains(&d.to_string_lossy().to_string()), "{:?}", r);
    // Put our own temp file back out of the real Trash and remove it.
    disko_core::ops::put_back(Path::new(&t.to), &rv).unwrap();
    assert!(rv.exists());
}

fn releases_of(fx: &Fx) -> Vec<Finding> {
    probes::versions::releases(&fx.clutter(vec![], vec![], vec![]))
}

#[test]
fn releases_never_touch_personal_folders_or_unknown_roots() {
    let fx = Fx::new("rel-personal");
    // The reproduced case: ~/bin/report → ~/Work/2025/report/bin/report.
    let target = fx.file("Work/2025/report/bin/report", MB);
    let photos = fx.file("Work/2019-photos.zip", 2 * MB);
    let taxes = fx.file("Work/2023 Taxes/return.pdf", 2 * MB);
    let clients = fx.file("Work/2024 Clients/list.xlsx", 2 * MB);
    fx.dir("bin");
    std::os::unix::fs::symlink(&target, fx.home.join("bin/report")).unwrap();
    // The same shape under Documents, and an unknown tool with release-like names.
    let doc = fx.file("Documents/Projects/2.0.0/bin/tool", MB);
    fx.file("Documents/Projects/1.0.0/bin/tool", 2 * MB);
    std::os::unix::fs::symlink(&doc, fx.home.join("bin/doctool")).unwrap();
    let unknown = fx.file(".local/share/sometool/versions/2.0.0", MB);
    fx.file(".local/share/sometool/versions/1.0.0", 2 * MB);
    fx.dir(".local/bin");
    std::os::unix::fs::symlink(&unknown, fx.home.join(".local/bin/sometool")).unwrap();
    assert!(releases_of(&fx).is_empty(), "{:#?}", releases_of(&fx));
    assert!(photos.exists() && taxes.exists() && clients.exists());
}

#[test]
fn releases_need_the_same_layout_as_the_one_in_use() {
    let fx = Fx::new("rel-layout");
    // Codex: releases/<v>/bin/codex, `current` → newest.
    let rel = ".codex/packages/standalone/releases";
    let cur = fx.dir(&format!("{}/0.169.0-aarch64-apple-darwin", rel));
    fx.file(&format!("{}/0.169.0-aarch64-apple-darwin/bin/codex", rel), MB);
    fx.file(&format!("{}/0.167.1-aarch64-apple-darwin/bin/codex", rel), 2 * MB);
    // Version-named, older, but not shaped like a release: a folder without bin/codex, and a file.
    fx.file(&format!("{}/0.150.0-notes/readme.txt", rel), 2 * MB);
    fx.file(&format!("{}/0.140.0-backup.tar", rel), 2 * MB);
    std::os::unix::fs::symlink(&cur, fx.home.join(".codex/packages/standalone/current")).unwrap();
    // Ardia: `current` → 0.2.46 beside non-release folders.
    let ardia = "Library/Application Support/Ardia/CLI";
    fx.file(&format!("{}/0.2.46/bin/ardia", ardia), MB);
    fx.file(&format!("{}/0.2.40/bin/ardia", ardia), 2 * MB);
    fx.file(&format!("{}/cua-driver/bin/x", ardia), 2 * MB);
    fx.file(&format!("{}/Usage/log", ardia), 2 * MB);
    std::os::unix::fs::symlink("0.2.46", fx.home.join(format!("{}/current", ardia))).unwrap();
    // Claude Code: each release is the executable itself; a folder named like a version isn't one.
    let claude = fx.file(".local/share/claude/versions/2.1.325", MB);
    fx.file(".local/share/claude/versions/2.1.321", 2 * MB);
    fx.file(".local/share/claude/versions/2.1.300/keep.txt", 2 * MB);
    fx.dir(".local/bin");
    std::os::unix::fs::symlink(&claude, fx.home.join(".local/bin/claude")).unwrap();

    let mut found = releases_of(&fx);
    found.sort_by(|a, b| a.title.cmp(&b.title));
    let summary: Vec<(String, Vec<String>)> = found.iter().map(|f| (f.title.clone(), f.paths.iter().map(|p| Path::new(p).file_name().unwrap().to_string_lossy().to_string()).collect())).collect();
    assert_eq!(
        summary,
        vec![
            ("Ardia".to_string(), vec!["0.2.40".to_string()]),
            ("Claude Code".to_string(), vec!["2.1.321".to_string()]),
            ("Codex CLI".to_string(), vec!["0.167.1-aarch64-apple-darwin".to_string()]),
        ]
    );
}

#[test]
fn admin_items_are_skipped_unless_safe_and_delete() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new("admin");
    let locked = fx.dir("locked");
    let review = fx.file("locked/Xcode.app/x", MB).parent().unwrap().to_path_buf();
    let trash = fx.file("locked/texlive-2023/x", MB).parent().unwrap().to_path_buf();
    // Our parent folder is not writable: removing needs an administrator.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let mut a = fake("t.review", vec![review.clone()], Safety::Review, Action::Delete);
    a.admin = true;
    let mut b = fake("t.trash", vec![trash.clone()], Safety::Safe, Action::Trash);
    b.admin = true;
    let c = fx.clutter(vec![], vec![], vec![]);
    let r = c.clean(&[a.clone(), b.clone()], &|_| false, &mut |_, _, _| {});
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(review.exists() && trash.exists());
    assert!(r.removed.is_empty() && r.trashed.is_empty() && r.deleted.is_empty() && r.freed == 0, "{:?}", r);
    for p in [&review, &trash] {
        let s = r.skipped.iter().find(|s| s.path == p.to_string_lossy()).expect("skipped");
        assert_eq!(s.error, disko_clutter::clean::NEEDS_ADMIN);
    }
}

#[test]
fn shared_commands_share_failure_and_keep() {
    let fx = Fx::new("shared");
    // A failing command shared by two findings: both fail, neither is removed.
    let x = fx.file("c/x/f", MB).parent().unwrap().to_path_buf();
    let y = fx.file("c/y/f", MB).parent().unwrap().to_path_buf();
    let fail = Action::Command { argv: vec!["/usr/bin/false".into()], label: "false".into() };
    let f1 = fake("t.f1", vec![x.clone()], Safety::Safe, fail.clone());
    let f2 = fake("t.f2", vec![y.clone()], Safety::Safe, fail);
    let c = fx.clutter(vec![], vec![], vec![]);
    let r = c.clean(&[f1.clone(), f2.clone()], &|_| false, &mut |_, _, _| {});
    assert!(r.removed.is_empty(), "{:?}", r);
    assert!(r.failed.iter().any(|f| f.path == x.to_string_lossy()) && r.failed.iter().any(|f| f.path == y.to_string_lossy()), "{:?}", r);
    assert_eq!(r.freed, 0);

    // A command that removes both: the second finding's path is kept, so it runs for neither.
    let rm = Action::Command { argv: vec!["/bin/rm".into(), "-rf".into(), x.to_string_lossy().to_string(), y.to_string_lossy().to_string()], label: "rm".into() };
    let g1 = fake("t.g1", vec![x.clone()], Safety::Safe, rm.clone());
    let g2 = fake("t.g2", vec![y.clone()], Safety::Safe, rm);
    let ys = y.to_string_lossy().to_string();
    let r = c.clean(&[g1.clone(), g2.clone()], &|p| p == ys, &mut |_, _, _| {});
    assert!(x.exists() && y.exists());
    assert!(r.removed.is_empty() && r.freed == 0, "{:?}", r);
    assert!(r.skipped.iter().any(|s| s.path == x.to_string_lossy()) && r.skipped.iter().any(|s| s.path == ys));

    // Nothing kept: it runs once and both are removed, with their paths recorded.
    let r = c.clean(&[g1.clone(), g2.clone()], &|_| false, &mut |_, _, _| {});
    assert!(!x.exists() && !y.exists());
    assert!(r.removed.contains(&g1.id) && r.removed.contains(&g2.id));
    assert_eq!(r.deleted.len(), 2, "{:?}", r);
    assert!(r.freed >= 2 * MB as u64);
}

#[test]
fn wrangler_state_is_not_build_output() {
    let fx = Fx::new("wrangler");
    fx.file("dev/site/package.json", 10);
    fx.file("dev/site/wrangler.toml", 10);
    fx.file("dev/site/.wrangler/state/v3/d1/db.sqlite", 2 * MB);
    fx.file("dev/site/.wrangler/tmp/bundle.js", 2 * MB);
    let c = fx.clutter(vec![], vec![], vec![]);
    assert!(fx.walk(&c).values().all(|f| !f.paths.iter().any(|p| p.contains(".wrangler"))));
}

#[test]
fn walk_matcher_reports_categories_for_claims() {
    use disko_core::WalkMatcher;
    let fx = Fx::new("claims");
    let c = fx.clutter(vec![], vec![], vec![]);
    let tag = c.on_dir(&fx.home.join(".npm/_cacache"), "_cacache", &["_cacache"]).unwrap();
    assert_eq!(c.category(tag), Category::Packages);
    assert!(c.on_matched(disko_core::Matched { tag, path: "x".into(), bytes: 1, files: 1, newest: None }).is_none());
    let t2 = c.on_dir(&fx.home.join("Library/Caches/com.foo"), "com.foo", &[]).unwrap();
    assert_eq!(c.category(t2), Category::Cache);
    assert!(c.on_dir(&fx.home.join("Library/Caches/com.apple.Safari"), "com.apple.Safari", &[]).is_none());
}

#[test]
fn build_output_parked_in_dot_cache_is_split_from_the_rest() {
    let fx = Fx::new("dotcache");
    fx.file(".cache/tools/target-a/CACHEDIR.TAG", 0);
    fs::write(fx.home.join(".cache/tools/target-a/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n# created by cargo\n").unwrap();
    fx.file(".cache/tools/target-a/debug/bin", 2 * MB);
    fx.file(".cache/tools/lib/python/x.so", 2 * MB);
    let c = fx.clutter(vec![], vec![], vec![]);
    let found = Mutex::new(vec![]);
    c.probes(&|v| found.lock().unwrap().extend(v));
    let found = found.into_inner().unwrap();
    let build: Vec<&Finding> = found.iter().filter(|f| f.rule == "build").collect();
    assert_eq!(build.len(), 1);
    assert_eq!(build[0].safety, Safety::Safe);
    assert!(build[0].paths[0].ends_with("target-a"));
    let rest: Vec<&Finding> = found.iter().filter(|f| f.rule == "cache.dot-cache").collect();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].safety, Safety::Review);
    assert!(rest[0].paths.iter().all(|p| !p.contains("target-a")));
    // The walk doesn't match inside: the folder is claimed by the probe.
    assert!(fx.walk(&c).values().all(|f| !f.paths.iter().any(|p| p.contains("/.cache/"))));
}

#[test]
fn texlive_years_are_four_digit_folders_only() {
    let fx = Fx::new("texlive");
    fx.file("Library/texlive/2023/x", 2 * MB);
    fx.file("Library/texlive/2024/x", 2 * MB);
    fx.file("Library/texlive/2022 Taxes/x", 2 * MB);
    fx.file("Library/texlive/2021-photos.zip", 2 * MB);
    let c = fx.clutter(vec![], vec![], vec![]);
    let found = probes::deep::texlive(&c);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].paths.len(), 1);
    assert!(found[0].paths[0].ends_with("/2023"), "{:?}", found[0].paths);
}

#[test]
fn check_path_needs_the_plain_form() {
    let dir = std::env::temp_dir().join(format!("disko-plain-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("x")).unwrap();
    let real = std::fs::canonicalize(&dir).unwrap();
    assert!(disko_clutter::clean::check_path(&real.join("x")).is_ok());
    assert!(disko_clutter::clean::check_path(std::path::Path::new(&format!("{}/./x", real.display()))).is_err());
    assert!(disko_clutter::clean::check_path(std::path::Path::new(&format!("{}//x", real.display()))).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
