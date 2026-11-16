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
    // A tool installed as versions with a launcher symlink.
    fx.file(".local/share/tool/versions/1.0.0", 2 * MB);
    let cur = fx.file(".local/share/tool/versions/2.0.0", 2 * MB);
    fx.file(".local/share/tool/versions/3.0.0-beta", 2 * MB);
    fx.dir(".local/bin");
    std::os::unix::fs::symlink(&cur, fx.home.join(".local/bin/tool")).unwrap();
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
    // Put our own temp file back out of the real Trash and remove it.
    disko_core::ops::put_back(Path::new(&t.to), &rv).unwrap();
    assert!(rv.exists());
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
