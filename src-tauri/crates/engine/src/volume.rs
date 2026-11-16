//! Where the space on the boot volume went, including what a file walk can't see: other APFS
//! volumes in the container (Preboot, Recovery, VM, Update …), local snapshots, purgeable space,
//! unreadable folders and the rest.
//!
//! `diskutil` is run at most once a minute (cached), with a 5 s timeout.

use disko_core::{HiddenSpace, VolumeInfo};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Default, Debug)]
pub struct Apfs {
    pub name: String,
    /// Other volumes in the same container: (role, name, bytes in use).
    pub others: Vec<(String, String, u64)>,
    pub snapshots: usize,
}

#[derive(Default)]
pub struct Cache {
    at: Option<Instant>,
    root: String,
    apfs: Apfs,
}

impl Cache {
    pub fn apfs(&mut self, root: &str) -> Apfs {
        if self.root != root || self.at.map(|t| t.elapsed() > Duration::from_secs(60)).unwrap_or(true) {
            self.apfs = probe_apfs(root);
            self.at = Some(Instant::now());
            self.root = root.to_string();
        }
        self.apfs.clone()
    }
}

/// (total, free, device id "disk3s5") for the volume holding `path`.
pub fn statfs(path: &str) -> Option<(u64, u64, String)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let bs = s.f_bsize as u64;
    let from = unsafe { std::ffi::CStr::from_ptr(s.f_mntfromname.as_ptr()) }.to_string_lossy().into_owned();
    Some((s.f_blocks * bs, s.f_bavail * bs, from.trim_start_matches("/dev/").to_string()))
}

/// Run a command with a timeout, returning stdout.
pub fn run(argv: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let buf = reader.join().ok()?;
                return st.success().then_some(buf);
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn probe_apfs(root: &str) -> Apfs {
    let mut a = Apfs::default();
    // The walked volumes: the one holding the root, and on the boot disk the data volume too.
    let mut walked: Vec<String> = Vec::new();
    if let Some((_, _, dev)) = statfs(root) {
        walked.push(dev);
    }
    if root == "/" {
        if let Some((_, _, dev)) = statfs("/System/Volumes/Data") {
            walked.push(dev);
        }
    }
    // "disk3s1s1" (sealed system snapshot) belongs to volume "disk3s1".
    let is_walked = |id: &str| walked.iter().any(|w| w == id || w.strip_prefix(id).map(|r| r.starts_with('s')).unwrap_or(false));
    let Some(out) = run(&["/usr/sbin/diskutil", "apfs", "list", "-plist"], Duration::from_secs(5)) else { return a };
    let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(out)) else { return a };
    let containers = v.as_dictionary().and_then(|d| d.get("Containers")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let mut data_dev = None;
    for c in containers {
        let Some(vols) = c.as_dictionary().and_then(|d| d.get("Volumes")).and_then(|v| v.as_array()) else { continue };
        let id = |v: &plist::Value| v.as_dictionary().and_then(|d| d.get("DeviceIdentifier")).and_then(|x| x.as_string()).unwrap_or("").to_string();
        if !vols.iter().any(|v| is_walked(&id(v))) {
            continue;
        }
        for v in vols {
            let Some(d) = v.as_dictionary() else { continue };
            let dev = id(v);
            let name = d.get("Name").and_then(|x| x.as_string()).unwrap_or("").to_string();
            let role = d.get("Roles").and_then(|r| r.as_array()).and_then(|r| r.first()).and_then(|r| r.as_string()).unwrap_or("").to_string();
            let used = d.get("CapacityInUse").and_then(|x| x.as_unsigned_integer()).unwrap_or(0);
            if is_walked(&dev) {
                if role == "Data" || (data_dev.is_none() && role != "System") {
                    data_dev = Some(dev.clone());
                }
                if role == "System" || a.name.is_empty() {
                    a.name = name.trim_end_matches(" - Data").to_string();
                }
            } else {
                a.others.push((role, name, used));
            }
        }
        break;
    }
    if let Some(dev) = data_dev {
        if let Some(out) = run(&["/usr/sbin/diskutil", "apfs", "listSnapshots", "-plist", &dev], Duration::from_secs(5)) {
            if let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(out)) {
                a.snapshots = v.as_dictionary().and_then(|d| d.get("Snapshots")).and_then(|s| s.as_array()).map(|s| s.len()).unwrap_or(0);
            }
        }
    }
    a
}

fn gb(b: u64) -> String {
    if b >= 1_000_000_000 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{} MB", b / 1_000_000)
    }
}

/// Put the pieces together. `scanned` = bytes the walk found; `unreadable` = folders it couldn't
/// read; `fda` = whether Disko has Full Disk Access.
pub fn info(root: &str, apfs: &Apfs, scanned: u64, unreadable: u64, fda: bool) -> VolumeInfo {
    let (total, free, _) = statfs(root).unwrap_or((0, 0, String::new()));
    let used = total.saturating_sub(free);
    let purgeable = crate::ffi::important_available(root).map(|a| a.saturating_sub(free)).unwrap_or(0);
    let mut hidden = Vec::new();
    let mut gap = used.saturating_sub(scanned);

    let vm: u64 = apfs.others.iter().filter(|o| o.0 == "VM").map(|o| o.2).sum();
    let sys: Vec<&(String, String, u64)> = apfs.others.iter().filter(|o| o.0 != "VM" && o.2 > 0).collect();
    let sys_bytes: u64 = sys.iter().map(|o| o.2).sum();
    if sys_bytes > 0 {
        let parts: Vec<String> = sys.iter().filter(|o| o.2 >= 10_000_000).map(|o| format!("{} {}", if o.0.is_empty() { &o.1 } else { &o.0 }, gb(o.2))).collect();
        hidden.push(HiddenSpace {
            kind: "system".into(),
            label: "macOS support volumes".into(),
            bytes: sys_bytes,
            why: format!("{}. macOS manages these; they aren't files you can see.", parts.join(", ")),
        });
        gap = gap.saturating_sub(sys_bytes);
    }
    if vm > 0 {
        hidden.push(HiddenSpace { kind: "vm".into(), label: "Swap and sleep image".into(), bytes: vm, why: "Memory macOS swapped to disk. It shrinks after a restart.".into() });
        gap = gap.saturating_sub(vm);
    }
    if purgeable > 0 && gap > 0 {
        let b = purgeable.min(gap);
        let (kind, label) = if apfs.snapshots > 0 {
            ("snapshots", format!("{} local snapshot{} and purgeable space", apfs.snapshots, if apfs.snapshots == 1 { "" } else { "s" }))
        } else {
            ("purgeable", "Purgeable space".to_string())
        };
        hidden.push(HiddenSpace { kind: kind.into(), label, bytes: b, why: "Time Machine snapshots and caches macOS frees by itself when space runs low.".into() });
        gap -= b;
    } else if apfs.snapshots > 0 {
        hidden.push(HiddenSpace {
            kind: "snapshots".into(),
            label: format!("{} local snapshot{}", apfs.snapshots, if apfs.snapshots == 1 { "" } else { "s" }),
            bytes: gap,
            why: "Snapshots keep deleted files until macOS removes them (estimate: space no file accounts for).".into(),
        });
        gap = 0;
    }
    if unreadable > 0 {
        hidden.push(HiddenSpace {
            kind: "unreadable".into(),
            label: format!("{} folder{} Disko couldn't read", unreadable, if unreadable == 1 { "" } else { "s" }),
            bytes: gap,
            why: if fda { "Protected by macOS or owned by another user (estimate, includes file system overhead).".into() } else { "Give Disko Full Disk Access to see inside (estimate, includes file system overhead).".into() },
        });
        gap = 0;
    }
    if gap > 0 {
        hidden.push(HiddenSpace { kind: "other".into(), label: "Everything else".into(), bytes: gap, why: "File system overhead, cloud folders Disko doesn't scan, and changes since the scan.".into() });
    }
    VolumeInfo { name: if apfs.name.is_empty() { "Macintosh HD".into() } else { apfs.name.clone() }, total, used, free, purgeable, scanned, hidden }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_adds_up() {
        let apfs = Apfs { name: "Mac".into(), others: vec![("Preboot".into(), "Preboot".into(), 5_000_000_000), ("VM".into(), "VM".into(), 2_000_000_000)], snapshots: 0 };
        let v = info("/", &apfs, 1_000_000, 3, true);
        assert!(v.total > 0 && v.used > 0);
        let hidden: u64 = v.hidden.iter().map(|h| h.bytes).sum();
        // Every byte of used is either scanned or explained (unless scanned exceeds used).
        assert_eq!(hidden + v.scanned, v.used.max(v.scanned + 7_000_000_000));
        assert!(v.hidden.iter().any(|h| h.kind == "unreadable"));
    }
}
