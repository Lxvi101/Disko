//! Read-only look at this Mac: top 20 apps by total size and the agent-session tree with totals
//! compared to `du` of each tool's folders. Removes nothing.
//!
//!     cargo run -p disko-apps --example inventory -j 4 [--release] [-- --json]

use std::time::Instant;

fn gb(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.2} GB", b as f64 / 1e9)
    } else {
        format!("{:.1} MB", b as f64 / 1e6)
    }
}

fn du(path: &str) -> Option<u64> {
    let out = std::process::Command::new("/usr/bin/du").args(["-sk", path]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next()?.parse::<u64>().ok().map(|k| k * 1024)
}

fn main() {
    let json = std::env::args().any(|a| a == "--json");
    let t = Instant::now();
    let apps = disko_apps::apps();
    let apps_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let _ = disko_apps::apps();
    let apps_cached_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let tools = disko_apps::sessions();
    let sessions_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let _ = disko_apps::sessions();
    let sessions_cached_ms = t.elapsed().as_millis();

    if json {
        println!("{}", serde_json::json!({ "apps": apps, "sessions": tools }));
        return;
    }

    println!("== Apps: {} found in {} ms (cached repeat {} ms) ==", apps.len(), apps_ms, apps_cached_ms);
    for a in apps.iter().take(20) {
        let flags = [
            a.running.then_some("running"),
            a.apple.then_some("apple"),
            a.admin.then_some("admin"),
            a.uninstaller.as_ref().map(|_| "vendor-uninstaller"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",");
        let last = a.last_used.map(|t| format!("{}d ago", (disko_apps_now() - t) / 86400)).unwrap_or_else(|| "?".into());
        println!(
            "{:>10}  {:<34} app {:>10}  data {:>10} ({:>2} paths, {} shared)  {:<12} last used {:<8} {}",
            gb(a.app_bytes + a.data_bytes),
            a.name.chars().take(34).collect::<String>(),
            gb(a.app_bytes),
            gb(a.data_bytes),
            a.data_paths.len(),
            a.shared_paths.len(),
            a.source,
            last,
            flags
        );
    }

    println!("\n== Agent sessions in {} ms (cached repeat {} ms) ==", sessions_ms, sessions_cached_ms);
    for t in &tools {
        let n: usize = t.projects.iter().map(|p| p.sessions.len()).sum();
        let shared: u64 = t.shared.iter().map(|s| s.bytes).sum();
        let du_total: u64 = t.roots.iter().filter_map(|r| du(r)).sum();
        println!(
            "\n{} — total {} (du of {} = {})  sessions {} in {} projects / {} sessions  shared {}  parts sum {}",
            t.name,
            gb(t.bytes),
            t.roots.join(", "),
            gb(du_total),
            gb(t.session_bytes),
            t.projects.len(),
            n,
            gb(shared),
            gb(t.session_bytes + shared)
        );
        for p in t.projects.iter().take(8) {
            println!("   {:>10}  {:<40} {} sessions", gb(p.bytes), p.name.chars().take(40).collect::<String>(), p.sessions.len());
            for s in p.sessions.iter().take(3) {
                println!(
                    "        {:>10}  [{}] {}{}{}",
                    gb(s.bytes),
                    s.kind,
                    s.title.clone().unwrap_or_else(|| s.id.clone()).chars().take(60).collect::<String>(),
                    s.busy.as_ref().map(|b| format!("  (busy: {b})")).unwrap_or_default(),
                    s.warning.as_ref().map(|w| format!("  (! {w})")).unwrap_or_default()
                );
            }
        }
        for s in t.shared.iter().take(8) {
            println!("   shared {:>10}  {}{}", gb(s.bytes), s.label, if s.paths.is_empty() { "" } else { "  (removable)" });
        }
    }
}

fn disko_apps_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
