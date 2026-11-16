//! Small shared helpers: process list, bounded file reads, read-only SQLite, timestamps.

use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

pub fn children(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// Directories only, symlinks excluded.
pub fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

pub fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// Run a program with a timeout; stdout on success (whatever the exit code), None on timeout.
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> Option<(String, String)> {
    run_env(cmd, args, &[], timeout)
}

/// `run` with extra environment variables.
pub fn run_env(cmd: &str, args: &[&str], envs: &[(&str, &str)], timeout: Duration) -> Option<(String, String)> {
    let mut child = Command::new(cmd).args(args).envs(envs.iter().copied()).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null()).spawn().ok()?;
    let start = Instant::now();
    // Drain pipes on threads so a chatty child can't block on a full pipe.
    let mut so = child.stdout.take()?;
    let mut se = child.stderr.take()?;
    let t1 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let t2 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    let out = String::from_utf8_lossy(&t1.join().unwrap_or_default()).to_string();
    let err = String::from_utf8_lossy(&t2.join().unwrap_or_default()).to_string();
    Some((out, err))
}

/// Executable paths of all running processes (full paths where the kernel knows them).
pub fn process_paths() -> Vec<String> {
    run("/bin/ps", &["-axww", "-o", "comm="], Duration::from_secs(5))
        .map(|(o, _)| o.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

pub fn pid_alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks for existence/permission.
    pid > 0 && (unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// First `n` bytes of a file.
pub fn head(path: &Path, n: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = fs::File::open(path) {
        let _ = f.take(n as u64).read_to_end(&mut buf);
    }
    buf
}

/// Last `n` bytes of a file.
pub fn tail(path: &Path, n: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(mut f) = fs::File::open(path) {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if f.seek(SeekFrom::Start(len.saturating_sub(n))).is_ok() {
            let _ = f.read_to_end(&mut buf);
        }
    }
    buf
}

/// Complete JSON lines in a byte buffer (a partial first/last line is dropped by parse failure).
pub fn json_lines(buf: &[u8]) -> impl Iterator<Item = serde_json::Value> + '_ {
    buf.split(|b| *b == b'\n').filter_map(|l| serde_json::from_slice(l).ok())
}

/// Find `"key":"value"` in raw text without parsing the (possibly huge, truncated) JSON.
pub fn raw_string_field(buf: &[u8], key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(buf);
    let needle = format!("\"{}\":\"", key);
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// Open a SQLite database without ever writing to it or blocking its owner. Tries a normal
/// read-only open first (sees the WAL), then `immutable=1` (works without -shm access).
pub fn open_ro(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let enc = path.to_string_lossy().replace('%', "%25").replace('?', "%3f").replace('#', "%23");
    for uri in [format!("file:{}?mode=ro", enc), format!("file:{}?mode=ro&immutable=1", enc)] {
        if let Ok(c) = Connection::open_with_flags(&uri, flags) {
            let _ = c.busy_timeout(Duration::from_millis(250));
            if c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_ok() {
                return Some(c);
            }
        }
    }
    None
}

/// Parse "2026-10-17T11:30:07.812Z", "2026-11-16 12:11:46.213+00:00", "2026-10-17" → unix s.
pub fn parse_time(t: &str) -> Option<i64> {
    let t = t.trim();
    let num = |a: usize, b: usize| t.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = if t.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    // Offset: trailing Z, or ±HH:MM.
    let mut off = 0i64;
    if let Some(pos) = t[10.min(t.len())..].rfind(['+', '-']).map(|p| p + 10.min(t.len())) {
        if pos > 18 {
            let sign = if &t[pos..pos + 1] == "-" { -1 } else { 1 };
            let oh = t.get(pos + 1..pos + 3).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
            let om = t.get(pos + 4..pos + 6).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
            off = sign * (oh * 3600 + om * 60);
        }
    }
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se - off)
}

/// Seconds or milliseconds → seconds.
pub fn secs(v: i64) -> i64 {
    if v > 100_000_000_000 {
        v / 1000
    } else {
        v
    }
}

/// One line, at most `max` chars, no markup noise.
pub fn clean_title(t: &str, max: usize) -> Option<String> {
    let line = t.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c: char| c == '#' || c == '*' || c.is_whitespace());
    if line.is_empty() {
        return None;
    }
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    Some(out)
}

#[cfg(test)]
pub fn test_dir(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let p = std::env::temp_dir().join(format!("disko-apps-{}-{}-{}", name, std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    fs::canonicalize(p).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-10-17T11:30:07.812Z"), Some(1792236607));
        assert_eq!(parse_time("2026-11-16 12:11:46.213632+00:00"), Some(1794831106));
        assert_eq!(parse_time("2026-11-16T13:11:46+01:00"), Some(1794831106));
        assert_eq!(secs(1794831106000), 1794831106);
    }

    #[test]
    fn raw_fields_survive_truncation() {
        let buf = br#"{"payload":{"id":"x","cwd":"/Users/a/b \"q\"","base_instructions":{"text":"very long trunc"#;
        assert_eq!(raw_string_field(buf, "cwd").as_deref(), Some("/Users/a/b \"q\""));
        assert_eq!(raw_string_field(buf, "text"), None);
    }

    #[test]
    fn titles() {
        assert_eq!(clean_title("\n  # Fix the build \nmore", 40).as_deref(), Some("Fix the build"));
        assert_eq!(clean_title("abcdef", 3).as_deref(), Some("abc…"));
    }
}
