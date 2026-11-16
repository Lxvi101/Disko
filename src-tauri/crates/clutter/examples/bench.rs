//! The clutter benchmark: how much clutter does Disko find on this Mac? Read-only.
//!
//!   cargo run -p disko-clutter --example bench --release -j 4 [-- --walk] [--json out.json] [--top 60]
//!
//! Runs every probe (and with `--walk`, a full walk of the home folder through the same matcher
//! the engine uses), then prints a table per category, the biggest findings, and how much of it
//! Disko 0.6's Quick Clean would have found (by rule family).

use disko_clutter::{walk, Clutter};
use disko_core::{Category, Finding, Safety};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Rule families Disko 0.6 Quick Clean also covered (see legacy/rust/quick_clean.rs).
fn legacy_covers(f: &Finding) -> bool {
    let r = f.rule.as_str();
    matches!(
        r,
        "packages.npm" | "packages.pnpm" | "packages.yarn" | "packages.bun" | "packages.build-tools" | "packages.uv" | "packages.pip" | "packages.poetry" | "packages.cargo" | "packages.go-build"
            | "packages.gradle" | "packages.homebrew" | "packages.cocoapods" | "packages.swiftpm" | "extras.huggingface" | "versions.playwright" | "developer.derived-data"
            | "developer.xcode-caches" | "developer.simulator-caches" | "developer.simulators-unavailable" | "developer.device-support" | "containers.build-cache" | "containers.images"
            | "trash" | "logs.app" | "logs.apps" | "cache.app"
    ) || (r.starts_with("build") && r != "build.environments" || r == "build.environments")
        || (r == "installers.file" && f.paths.iter().all(|p| p.contains("/Downloads/")))
        || (r == "versions.releases" && (f.title == "Claude Code" || f.title == "Cursor Agent"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let do_walk = args.iter().any(|a| a == "--walk");
    let json = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)).cloned();
    let top: usize = args.iter().position(|a| a == "--top").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(50);

    let clutter = Clutter::new(vec![]);
    eprintln!("Full Disk Access: {}", clutter.env.fda);
    let found: Mutex<HashMap<String, Finding>> = Mutex::new(HashMap::new());
    let t0 = Instant::now();
    let first = Mutex::new(None);
    clutter.probes(&|fs: Vec<Finding>| {
        let mut f = first.lock().unwrap();
        if f.is_none() {
            *f = Some(t0.elapsed());
        }
        let mut m = found.lock().unwrap();
        for x in fs {
            m.insert(x.id.clone(), x);
        }
    });
    let probes_ms = t0.elapsed().as_millis();
    eprintln!("probes: {} findings in {} ms (first after {:?})", found.lock().unwrap().len(), probes_ms, first.lock().unwrap().unwrap_or_default());
    if do_walk {
        let t1 = Instant::now();
        let st = walk::walk(&clutter, &clutter.env.home, &|f| {
            found.lock().unwrap().insert(f.id.clone(), f);
        });
        eprintln!(
            "walk of {}: {} dirs, {} files, {:.1} GB in {:.1} s",
            clutter.env.home.display(),
            st.dirs.load(std::sync::atomic::Ordering::Relaxed),
            st.files.load(std::sync::atomic::Ordering::Relaxed),
            st.bytes.load(std::sync::atomic::Ordering::Relaxed) as f64 / GB,
            t1.elapsed().as_secs_f64()
        );
    }
    let mut all: Vec<Finding> = found.into_inner().unwrap().into_values().collect();
    all.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    // Count each path once even if two findings overlap (a probe's folder and a walk match inside).
    let mut paths: Vec<(String, usize)> = all.iter().enumerate().flat_map(|(i, f)| f.paths.iter().map(move |p| (p.clone(), i))).collect();
    paths.sort();
    let mut overlapped = 0usize;
    let mut prev: Option<String> = None;
    for (p, _) in &paths {
        if let Some(q) = &prev {
            if p.starts_with(&format!("{}/", q)) {
                overlapped += 1;
                continue;
            }
        }
        prev = Some(p.clone());
    }

    let mut by_cat: BTreeMap<Category, (usize, u64, u64)> = BTreeMap::new();
    for f in &all {
        let e = by_cat.entry(f.category).or_default();
        e.0 += 1;
        e.1 += f.bytes;
        if f.safety == Safety::Safe && f.busy.is_none() {
            e.2 += f.bytes;
        }
    }
    println!("\n{:<14} {:>6} {:>10} {:>12}", "category", "count", "GB", "one-tap GB");
    println!("{}", "-".repeat(46));
    let (mut n, mut b, mut safe) = (0, 0u64, 0u64);
    for (c, (cn, cb, cs)) in &by_cat {
        println!("{:<14} {:>6} {:>10.2} {:>12.2}", format!("{:?}", c), cn, *cb as f64 / GB, *cs as f64 / GB);
        n += cn;
        b += cb;
        safe += cs;
    }
    println!("{}", "-".repeat(46));
    println!("{:<14} {:>6} {:>10.2} {:>12.2}", "TOTAL", n, b as f64 / GB, safe as f64 / GB);
    let legacy: u64 = all.iter().filter(|f| legacy_covers(f)).map(|f| f.bytes).sum();
    let legacy_n = all.iter().filter(|f| legacy_covers(f)).count();
    println!("\n0.6-equivalent rule families: {} findings, {:.2} GB · new in 1.0: {} findings, {:.2} GB", legacy_n, legacy as f64 / GB, n - legacy_n, (b - legacy) as f64 / GB);
    println!("busy now: {} · need admin: {} · overlapping paths skipped in totals: {}", all.iter().filter(|f| f.busy.is_some()).count(), all.iter().filter(|f| f.admin).count(), overlapped);

    println!("\nTop {} findings:", top.min(all.len()));
    for f in all.iter().take(top) {
        println!(
            "{:>8.2} GB  {:<11} {:<30} {:<28} {:<6} {}{}",
            f.bytes as f64 / GB,
            format!("{:?}", f.category),
            truncate(&f.title, 30),
            truncate(&f.rule, 28),
            if f.safety == Safety::Safe { "safe" } else { "review" },
            truncate(&f.why, 90),
            f.busy.as_ref().map(|b| format!("  [busy: {}]", b)).unwrap_or_default()
        );
    }
    if let Some(path) = json {
        std::fs::write(&path, serde_json::to_string_pretty(&all).unwrap()).unwrap();
        eprintln!("wrote {}", path);
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
