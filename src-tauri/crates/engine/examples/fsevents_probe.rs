//! Which paths FSEvents reports for "/" (firmlinked or data-volume form), and whether history
//! replay (sinceWhen) works there. Writes and deletes one file in /tmp.
fn main() {
    let f = format!("/tmp/disko-fsevents-probe-{}", std::process::id());
    let id = disko_engine::debug_current_event_id();
    let live = disko_engine::debug_raw_events("/", None, 3.0, || {
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(format!("{}/x", f), b"x").unwrap();
    });
    println!("live events on / :");
    for (p, fl, i) in live.iter().filter(|e| e.0.contains("disko-fsevents-probe")) {
        println!("  {:#08x} {} {}", fl, i, p);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    for root in ["/", "/System/Volumes/Data"] {
        let replay = disko_engine::debug_raw_events(root, Some(id), 3.0, || {});
        println!("replayed since {} on {} : {} events total", id, root, replay.len());
        for (p, fl, i) in replay.iter().filter(|e| e.0.contains("disko-fsevents-probe") || e.1 & 0x10 != 0 || e.1 & 0x1 != 0) {
            println!("  {:#08x} {} {}", fl, i, p);
        }
    }
    let live = disko_engine::debug_raw_events("/System/Volumes/Data", None, 3.0, || {
        std::fs::create_dir_all(&f).unwrap();
    });
    println!("live events on /System/Volumes/Data :");
    for (p, fl, i) in live.iter().filter(|e| e.0.contains("disko-fsevents-probe")) {
        println!("  {:#08x} {} {}", fl, i, p);
    }
    let _ = std::fs::remove_dir_all(&f);
}
