//! Engine tests on temp-dir fixtures (never the real disk).

use disko_core::{Category, Matched, WalkMatcher};
use disko_engine::{Engine, EngineConfig, EngineEvent};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIB: usize = 1 << 20;
static N: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    root: PathBuf,
    snap: PathBuf,
    base: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!("disko-engine-it-{}-{}-{}", tag, std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root")).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    Fixture { root: base.join("root"), snap: base.join("cache/snapshot.bin"), base }
}

fn write(p: &Path, bytes: usize) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    // Non-zero, varied data so nothing gets compressed or sparse.
    let data: Vec<u8> = (0..bytes).map(|i| (i % 251) as u8 + 1).collect();
    std::fs::write(p, data).unwrap();
}

/// Allocated bytes and file count on disk, hard links once.
fn du(p: &Path) -> (u64, u64) {
    fn rec(p: &Path, seen: &mut std::collections::HashSet<(u64, u64)>, out: &mut (u64, u64)) {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            let m = std::fs::symlink_metadata(e.path()).unwrap();
            if m.is_dir() {
                rec(&e.path(), seen, out);
            } else {
                out.1 += 1;
                if m.nlink() > 1 && !seen.insert((m.dev(), m.ino())) {
                    continue;
                }
                out.0 += m.blocks() * 512;
            }
        }
    }
    let mut out = (0, 0);
    rec(p, &mut Default::default(), &mut out);
    out
}

struct Rules {
    dirs_seen: Mutex<Vec<String>>,
}

impl Rules {
    fn new() -> Arc<Rules> {
        Arc::new(Rules { dirs_seen: Mutex::new(Vec::new()) })
    }
}

impl WalkMatcher for Rules {
    fn on_dir(&self, path: &Path, name: &str, siblings: &[&str]) -> Option<u32> {
        self.dirs_seen.lock().unwrap().push(path.to_string_lossy().to_string());
        match name {
            "node_modules" => Some(1),
            "target" if siblings.contains(&"Cargo.toml") => Some(2),
            _ => None,
        }
    }
    fn on_file(&self, _: &Path, name: &str, _: u64, _: i64) -> Option<u32> {
        name.ends_with(".dmg").then_some(3)
    }
    fn category(&self, tag: u32) -> Category {
        match tag {
            3 => Category::Installers,
            _ => Category::Build,
        }
    }
}

fn engine(f: &Fixture) -> Arc<Engine> {
    Engine::with_config(f.snap.clone(), EngineConfig { root: f.root.clone(), threads: 2, assume_fda: Some(true), ..Default::default() })
}

#[derive(Default)]
struct Events {
    matched: Mutex<Vec<Matched>>,
    changed: Mutex<Vec<String>>,
    gone: Mutex<Vec<String>>,
    trees: AtomicUsize,
    progress_done: AtomicUsize,
}

impl Events {
    fn sink(self: &Arc<Self>) -> Arc<dyn Fn(EngineEvent) + Send + Sync> {
        let me = self.clone();
        Arc::new(move |e| match e {
            EngineEvent::Matched(m) => me.matched.lock().unwrap().push(m),
            EngineEvent::Changed(c) => me.changed.lock().unwrap().extend(c),
            EngineEvent::Gone(g) => me.gone.lock().unwrap().extend(g),
            EngineEvent::Tree(_) => {
                me.trees.fetch_add(1, Ordering::Relaxed);
            }
            EngineEvent::Progress(p) if p.phase == "done" => {
                me.progress_done.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        })
    }
}

fn scan(e: &Engine, rules: Arc<Rules>, ev: &Arc<Events>) -> disko_engine::ScanStats {
    let sink = ev.sink();
    e.scan(rules, Arc::new(AtomicBool::new(false)), &*sink).unwrap()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn build_tree(r: &Path) {
    write(&r.join("a/small1.txt"), 1000);
    write(&r.join("a/small2.txt"), 5000);
    write(&r.join("a/big.bin"), 3 * MIB);
    write(&r.join("a/deep/er/x.txt"), 100);
    write(&r.join("proj/Cargo.toml"), 10);
    write(&r.join("proj/target/debug/app"), 2 * MIB);
    write(&r.join("proj/target/debug/node_modules/x.js"), 10);
    write(&r.join("proj/node_modules/pkg/index.js"), 4000);
    write(&r.join("proj/node_modules/pkg/node_modules/inner/i.js"), 4000);
    write(&r.join("dl/Installer.dmg"), 2 * MIB);
    std::fs::create_dir_all(r.join("empty")).unwrap();
}

#[test]
fn totals_folding_and_views() {
    let f = fixture("totals");
    build_tree(&f.root);
    let e = engine(&f);
    let ev = Arc::new(Events::default());
    let st = scan(&e, Rules::new(), &ev);
    let (bytes, files) = du(&f.root);
    assert_eq!(st.bytes, bytes);
    assert_eq!(st.files, files);
    let root = e.tree(None, 0).unwrap();
    assert_eq!(root.bytes, bytes);
    assert_eq!(root.files, files);
    assert!(root.newest.is_some());
    // Big files are nodes, small ones are folded.
    let big = e.size_of(&s(&f.root.join("a/big.bin"))).unwrap();
    assert_eq!(big.kind, "file");
    assert_eq!(big.files, 1);
    assert!(e.size_of(&s(&f.root.join("a/small1.txt"))).is_none());
    let a = e.list(&s(&f.root.join("a"))).unwrap();
    let (abytes, afiles) = du(&f.root.join("a"));
    assert_eq!(a.bytes, abytes);
    assert_eq!(a.files, afiles);
    let kids = a.children.unwrap();
    assert_eq!(kids.iter().map(|k| k.bytes).sum::<u64>(), abytes);
    assert!(kids.iter().any(|k| k.kind == "other" && k.files == 2));
    // Largest first.
    assert!(kids.windows(2).all(|w| w[0].bytes >= w[1].bytes || w[1].kind == "other"));
    // Empty folders are kept.
    assert_eq!(e.size_of(&s(&f.root.join("empty"))).unwrap().bytes, 0);
    // Search & largest files.
    let hits = e.search("BIG", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, s(&f.root.join("a/big.bin")));
    let largest = e.largest_files(None, 2);
    assert_eq!(largest[0].path, s(&f.root.join("a/big.bin")));
    let under = e.largest_files(Some(&s(&f.root.join("dl"))), 5);
    assert_eq!(under.len(), 1);
    // Final events.
    assert!(ev.trees.load(Ordering::Relaxed) >= 1);
    assert_eq!(ev.progress_done.load(Ordering::Relaxed), 1);
    assert!(ev.changed.lock().unwrap().contains(&s(&f.root)));
}

#[test]
fn matches_complete_and_dont_nest() {
    let f = fixture("match");
    build_tree(&f.root);
    let e = engine(&f);
    let ev = Arc::new(Events::default());
    let rules = Rules::new();
    scan(&e, rules.clone(), &ev);
    let m = ev.matched.lock().unwrap().clone();
    let by = |p: &str| m.iter().find(|x| x.path == s(&f.root.join(p))).cloned();
    let nm = by("proj/node_modules").expect("node_modules matched");
    assert_eq!(nm.tag, 1);
    assert_eq!((nm.bytes, nm.files), du(&f.root.join("proj/node_modules")));
    let tg = by("proj/target").expect("target next to Cargo.toml");
    assert_eq!(tg.bytes, du(&f.root.join("proj/target")).0);
    let dmg = by("dl/Installer.dmg").expect("dmg file");
    assert_eq!(dmg.tag, 3);
    // Nothing below a match is offered to the matcher or matched.
    assert!(by("proj/node_modules/pkg/node_modules").is_none());
    assert!(by("proj/target/debug/node_modules").is_none());
    let seen = rules.dirs_seen.lock().unwrap();
    assert!(!seen.iter().any(|d| d.contains("node_modules/pkg")));
    assert!(!seen.iter().any(|d| d.contains("target/debug")));
    // Views carry the category to everything inside.
    let inner = e.size_of(&s(&f.root.join("proj/node_modules/pkg"))).unwrap();
    assert_eq!(inner.clutter, Some(Category::Build));
    assert_eq!(e.size_of(&s(&f.root.join("dl/Installer.dmg"))).unwrap().clutter, Some(Category::Installers));
    assert_eq!(e.size_of(&s(&f.root.join("a"))).unwrap().clutter, None);
    let mm = e.matched_at(&[s(&f.root.join("proj/node_modules/pkg/index.js"))]);
    assert_eq!(mm.len(), 1);
    assert_eq!(mm[0].path, s(&f.root.join("proj/node_modules")));
}

#[test]
fn hard_links_count_once() {
    let f = fixture("links");
    write(&f.root.join("x/data.bin"), 2 * MIB);
    write(&f.root.join("x/small.txt"), 3000);
    std::fs::create_dir_all(f.root.join("y")).unwrap();
    std::fs::hard_link(f.root.join("x/data.bin"), f.root.join("y/data-link.bin")).unwrap();
    std::fs::hard_link(f.root.join("x/small.txt"), f.root.join("y/small-link.txt")).unwrap();
    let e = engine(&f);
    let st = scan(&e, Rules::new(), &Arc::new(Events::default()));
    let (bytes, _) = du(&f.root);
    assert_eq!(st.bytes, bytes);
    let x = e.size_of(&s(&f.root.join("x"))).unwrap().bytes;
    let y = e.size_of(&s(&f.root.join("y"))).unwrap().bytes;
    assert_eq!(x + y, bytes);
    assert!(x == 0 || y == 0 || x + y == bytes);
    // Refreshing a folder doesn't count its links a second time.
    e.refresh(&[s(&f.root.join("x")), s(&f.root.join("y"))], &*Rules::new(), &|_| {});
    assert_eq!(e.tree(None, 0).unwrap().bytes, bytes);
}

#[test]
fn removed_is_instant_and_exact() {
    let f = fixture("removed");
    build_tree(&f.root);
    let e = engine(&f);
    scan(&e, Rules::new(), &Arc::new(Events::default()));

    // A folder.
    std::fs::remove_dir_all(f.root.join("proj/node_modules")).unwrap();
    let t = Instant::now();
    let changed = e.removed(&[s(&f.root.join("proj/node_modules"))]);
    assert!(t.elapsed() < Duration::from_millis(50));
    assert_eq!(changed, vec![s(&f.root.join("proj"))]);
    assert_eq!(e.tree(None, 0).unwrap().bytes, du(&f.root).0);
    assert_eq!(e.size_of(&s(&f.root.join("proj"))).unwrap().bytes, du(&f.root.join("proj")).0);
    assert!(e.size_of(&s(&f.root.join("proj/node_modules"))).is_none());
    assert!(e.search("node_modules", 10).iter().all(|n| !n.path.ends_with("proj/node_modules")));

    // A big file and a small (folded) one.
    std::fs::remove_file(f.root.join("a/big.bin")).unwrap();
    std::fs::remove_file(f.root.join("a/small2.txt")).unwrap();
    let changed = e.removed(&[s(&f.root.join("a/big.bin")), s(&f.root.join("a/small2.txt"))]);
    assert_eq!(changed, vec![s(&f.root.join("a"))]);
    let (b, n) = du(&f.root);
    let root = e.tree(None, 0).unwrap();
    assert_eq!((root.bytes, root.files), (b, n));

    // A path that is still there (removal failed) is re-listed, not dropped.
    e.removed(&[s(&f.root.join("dl/Installer.dmg"))]);
    assert!(e.size_of(&s(&f.root.join("dl/Installer.dmg"))).is_some());
    assert_eq!(e.tree(None, 0).unwrap().bytes, du(&f.root).0);
}

#[test]
fn refresh_picks_up_changes() {
    let f = fixture("refresh");
    build_tree(&f.root);
    let e = engine(&f);
    scan(&e, Rules::new(), &Arc::new(Events::default()));
    let ev = Arc::new(Events::default());
    let sink = ev.sink();

    // New subfolder tree (walked), new big file, a grown small file, a deleted folder.
    write(&f.root.join("a/new/sub/n.bin"), 2 * MIB);
    write(&f.root.join("a/new/sub/n.txt"), 100);
    write(&f.root.join("a/new/proj2/node_modules/m/i.js"), 100);
    write(&f.root.join("a/another.bin"), MIB + 10);
    write(&f.root.join("a/small1.txt"), 50_000);
    std::fs::remove_dir_all(f.root.join("a/deep")).unwrap();
    let changed = e.refresh(&[s(&f.root.join("a"))], &*Rules::new(), &*sink);
    assert_eq!(changed, vec![s(&f.root.join("a"))]);
    let (b, n) = du(&f.root);
    let root = e.tree(None, 0).unwrap();
    assert_eq!((root.bytes, root.files), (b, n));
    assert_eq!(e.size_of(&s(&f.root.join("a/new/sub/n.bin"))).unwrap().kind, "file");
    assert!(e.size_of(&s(&f.root.join("a/deep"))).is_none());
    assert!(ev.matched.lock().unwrap().iter().any(|m| m.path == s(&f.root.join("a/new/proj2/node_modules"))));

    // A path the tree doesn't know refreshes its nearest known ancestor.
    write(&f.root.join("dl/sub/x/y.bin"), 2 * MIB);
    e.refresh(&[s(&f.root.join("dl/sub/x"))], &*Rules::new(), &*sink);
    assert_eq!(e.tree(None, 0).unwrap().bytes, du(&f.root).0);

    // A matched folder deleted outside Disko is reported gone.
    std::fs::remove_dir_all(f.root.join("proj/node_modules")).unwrap();
    e.refresh(&[s(&f.root.join("proj"))], &*Rules::new(), &*sink);
    assert!(ev.gone.lock().unwrap().contains(&s(&f.root.join("proj/node_modules"))));
    assert_eq!(e.tree(None, 0).unwrap().bytes, du(&f.root).0);

    // Changes inside a matched folder update its totals.
    ev.matched.lock().unwrap().clear();
    write(&f.root.join("proj/target/debug/more.bin"), 2 * MIB);
    e.refresh(&[s(&f.root.join("proj/target/debug"))], &*Rules::new(), &*sink);
    let m = ev.matched.lock().unwrap().clone();
    let t = m.iter().find(|m| m.path == s(&f.root.join("proj/target"))).expect("updated target");
    assert_eq!(t.bytes, du(&f.root.join("proj/target")).0);
}

#[test]
fn snapshot_roundtrip() {
    let f = fixture("snap");
    build_tree(&f.root);
    let e = engine(&f);
    let st = scan(&e, Rules::new(), &Arc::new(Events::default()));
    assert!(st.snapshot_bytes > 0);
    assert!(f.snap.exists());
    // Removal + compaction survive too.
    std::fs::remove_dir_all(f.root.join("a/deep")).unwrap();
    e.removed(&[s(&f.root.join("a/deep"))]);
    e.save_snapshot().unwrap();
    let before = e.tree(None, 10).unwrap();
    let matched_before = e.matched_at(&[s(&f.root.join("proj/target/debug"))]);

    let e2 = engine(&f);
    let age = e2.load_snapshot().expect("snapshot loads");
    assert!(age < Duration::from_secs(60));
    assert!(!e2.needs_scan());
    assert_eq!(e2.tree(None, 10).unwrap(), before);
    assert_eq!(e2.matched_at(&[s(&f.root.join("proj/target/debug"))]), matched_before);
    // Everything the matcher tagged comes back from the snapshot alone.
    let all = e2.matched_under(None);
    let mut paths: Vec<String> = all.iter().map(|m| m.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, vec![s(&f.root.join("dl/Installer.dmg")), s(&f.root.join("proj/node_modules")), s(&f.root.join("proj/target"))]);
    assert_eq!(all, e.matched_under(None));
    assert_eq!(e2.matched_under(Some(&s(&f.root.join("proj")))).len(), 2);
    assert!(e2.matched_under(Some(&s(&f.root.join("a")))).is_empty());
    assert_eq!(e2.search("dmg", 5).len(), 1);

    // A corrupt file is ignored.
    let data = std::fs::read(&f.snap).unwrap();
    std::fs::write(&f.snap, &data[..data.len() / 2]).unwrap();
    assert!(engine(&f).load_snapshot().is_none());
    assert!(engine(&f).needs_scan());
}

#[test]
fn cancel_keeps_previous_tree() {
    let f = fixture("cancel");
    build_tree(&f.root);
    for i in 0..50 {
        write(&f.root.join(format!("many/d{}/f.txt", i)), 10);
    }
    let e = engine(&f);
    scan(&e, Rules::new(), &Arc::new(Events::default()));
    let before = e.tree(None, 0).unwrap();

    struct Cancelling(Arc<AtomicBool>, AtomicUsize);
    impl WalkMatcher for Cancelling {
        fn on_dir(&self, _: &Path, _: &str, _: &[&str]) -> Option<u32> {
            if self.1.fetch_add(1, Ordering::Relaxed) == 5 {
                self.0.store(true, Ordering::Relaxed);
            }
            None
        }
        fn on_file(&self, _: &Path, _: &str, _: u64, _: i64) -> Option<u32> {
            None
        }
        fn category(&self, _: u32) -> Category {
            Category::Cache
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    write(&f.root.join("later.bin"), 2 * MIB);
    let r = e.scan(Arc::new(Cancelling(cancel.clone(), AtomicUsize::new(0))), cancel, &|_| {});
    assert!(r.is_err());
    assert_eq!(e.tree(None, 0).unwrap(), before);
    // Already cancelled before starting.
    let r = e.scan(Rules::new(), Arc::new(AtomicBool::new(true)), &|_| {});
    assert!(r.is_err());
    assert_eq!(e.tree(None, 0).unwrap(), before);
}

#[test]
fn unreadable_folders_are_flagged() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture("perm");
    write(&f.root.join("locked/secret.txt"), 100);
    write(&f.root.join("open/x.txt"), 100);
    std::fs::set_permissions(f.root.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let e = engine(&f);
    let st = scan(&e, Rules::new(), &Arc::new(Events::default()));
    std::fs::set_permissions(f.root.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    if unsafe { libc::geteuid() } == 0 {
        return; // root reads everything
    }
    assert_eq!(st.unreadable, 1);
    let n = e.size_of(&s(&f.root.join("locked"))).unwrap();
    assert_eq!(n.error.as_deref(), Some("Permission denied"));
    // Once readable, a refresh fills it in and clears the flag.
    e.refresh(&[s(&f.root.join("locked"))], &*Rules::new(), &|_| {});
    let n = e.size_of(&s(&f.root.join("locked"))).unwrap();
    assert!(n.error.is_none());
    assert_eq!(n.files, 1);
}

#[test]
fn watch_applies_outside_changes() {
    let f = fixture("watch");
    build_tree(&f.root);
    let e = engine(&f);
    let ev = Arc::new(Events::default());
    scan(&e, Rules::new(), &ev);
    ev.changed.lock().unwrap().clear();
    let h = e.watch(Rules::new(), ev.sink());
    std::thread::sleep(Duration::from_millis(300));
    write(&f.root.join("a/from-outside.bin"), 2 * MIB);
    std::fs::remove_dir_all(f.root.join("proj/node_modules")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let want = du(&f.root).0;
    while Instant::now() < deadline && e.tree(None, 0).unwrap().bytes != want {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(h);
    assert_eq!(e.tree(None, 0).unwrap().bytes, want);
    assert!(e.size_of(&s(&f.root.join("a/from-outside.bin"))).is_some());
    assert!(ev.changed.lock().unwrap().contains(&s(&f.root.join("a"))));
    assert!(ev.gone.lock().unwrap().contains(&s(&f.root.join("proj/node_modules"))));
}

#[test]
fn relaunch_replays_changes_since_snapshot() {
    let f = fixture("replay");
    build_tree(&f.root);
    {
        let e = engine(&f);
        scan(&e, Rules::new(), &Arc::new(Events::default()));
    }
    // Changes while "Disko is closed".
    write(&f.root.join("dl/while-closed.bin"), 4 * MIB);
    std::fs::remove_file(f.root.join("a/big.bin")).unwrap();
    std::fs::remove_dir_all(f.root.join("proj/node_modules")).unwrap();
    write(&f.root.join("brand/new/dir.bin"), 2 * MIB);
    std::thread::sleep(Duration::from_millis(1500)); // let fseventsd record them

    let e = engine(&f);
    e.load_snapshot().unwrap();
    assert!(!e.needs_scan());
    let ev = Arc::new(Events::default());
    let h = e.watch(Rules::new(), ev.sink());
    let want = du(&f.root).0;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && e.tree(None, 0).unwrap().bytes != want {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(h);
    if e.tree(None, 0).unwrap().bytes != want {
        fn dump(n: &disko_core::TreeNode, d: usize) {
            eprintln!("{}{} {} {} {}", "  ".repeat(d), n.kind, n.bytes, n.files, n.path);
            for c in n.children.iter().flatten() {
                dump(c, d + 1);
            }
        }
        dump(&e.tree(None, 8).unwrap(), 0);
        eprintln!("changed {:?}\ngone {:?}", ev.changed.lock().unwrap(), ev.gone.lock().unwrap());
    }
    assert_eq!(e.tree(None, 0).unwrap().bytes, want);
    assert!(e.size_of(&s(&f.root.join("dl/while-closed.bin"))).is_some());
    assert!(e.size_of(&s(&f.root.join("a/big.bin"))).is_none());
    assert!(e.size_of(&s(&f.root.join("proj/node_modules"))).is_none());
    assert!(e.size_of(&s(&f.root.join("brand/new/dir.bin"))).is_some());
    assert!(ev.gone.lock().unwrap().contains(&s(&f.root.join("proj/node_modules"))));
}

#[test]
fn volume_explains_the_gap() {
    let f = fixture("volume");
    build_tree(&f.root);
    let e = engine(&f);
    scan(&e, Rules::new(), &Arc::new(Events::default()));
    let v = e.volume();
    assert!(v.total > 0 && v.used > 0 && v.total >= v.used);
    assert_eq!(v.scanned, du(&f.root).0);
    let hidden: u64 = v.hidden.iter().map(|h| h.bytes).sum();
    assert_eq!(hidden + v.scanned, v.used.max(hidden + v.scanned));
}

#[test]
fn apfs_clones_count_once() {
    let f = fixture("clones");
    write(&f.root.join("orig/data.bin"), 3 * MIB);
    std::fs::create_dir_all(f.root.join("copy")).unwrap();
    let st = std::process::Command::new("/bin/cp").arg("-c").arg(f.root.join("orig/data.bin")).arg(f.root.join("copy/data.bin")).status().unwrap();
    assert!(st.success());
    let one = std::fs::symlink_metadata(f.root.join("orig/data.bin")).unwrap().blocks() * 512;
    let e = engine(&f);
    let st = scan(&e, Rules::new(), &Arc::new(Events::default()));
    // du counts the data twice; the engine counts it once.
    assert_eq!(du(&f.root).0, 2 * one);
    assert!(st.bytes >= one && st.bytes < one + one / 10, "{} vs {}", st.bytes, one);
    // Refreshing either folder keeps it counted once.
    e.refresh(&[s(&f.root.join("copy")), s(&f.root.join("orig"))], &*Rules::new(), &|_| {});
    assert_eq!(e.tree(None, 0).unwrap().bytes, st.bytes);
    // Removing the folder that held the full count hands it to the clone on its next refresh.
    std::fs::remove_dir_all(f.root.join("orig")).unwrap();
    e.removed(&[s(&f.root.join("orig"))]);
    e.refresh(&[s(&f.root.join("copy"))], &*Rules::new(), &|_| {});
    let b = e.tree(None, 0).unwrap().bytes;
    assert!(b >= one / 2, "{} vs {}", b, one);
}
