//! Anonymous sharing of learned rules. A shared rule becomes a [`Submission`]: the pattern
//! generalized (home → `~`, names that look personal → `*`, a personal folder chain → `**`), the
//! size bucketed, title/why scrubbed. Nothing else: no paths beyond the pattern, no user or
//! machine identifiers, no timestamps. Submissions wait in `outbox/` (one JSON file each, visible
//! to the user) until [`submit_with`] sends them to a configured endpoint.

use crate::{short_hash, LearnedRule};
use disko_core::{Category, Safety};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    pub schema_version: u32,
    pub pattern: String,
    pub category: Category,
    pub title: String,
    pub why: String,
    pub regrows: bool,
    pub safety: Safety,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Disko version that produced it (helps reviewers; not identifying).
    pub disko: String,
}

/// Folder names that are generic tool/build/OS vocabulary (kept as-is outside `~/Library`).
const GENERIC: [&str; 40] = [
    "Library", "Caches", "Application Support", "Containers", "Group Containers", "Logs", "Developer", "Xcode",
    "DerivedData", "Archives", "CoreSimulator", "node_modules", "target", "build", "dist", "out", "vendor", "Pods",
    "venv", "env", "cache", "caches", "tmp", "temp", "debug", "release", "intermediates", "obj", "bin", "pkg",
    "deps", "packages", "Data", "Cache", "Code Cache", "GPUCache", "Service Worker", "CacheStorage", "storage", "logs",
];

/// Size bucket for a submission.
pub fn size_bucket(bytes: u64) -> String {
    match bytes {
        b if b < 100_000_000 => "<100 MB",
        b if b < 1_000_000_000 => "100 MB–1 GB",
        b if b < 10_000_000_000 => "1–10 GB",
        _ => ">10 GB",
    }
    .into()
}

fn looks_personal(seg: &str, user: &str) -> bool {
    let l = seg.to_lowercase();
    if user.len() >= 3 && l.contains(&user.to_lowercase()) {
        return true;
    }
    if seg.contains('@') {
        return true;
    }
    // UUIDs, hashes, long numbers.
    let hex_run = seg.split(|c: char| !c.is_ascii_hexdigit()).map(str::len).max().unwrap_or(0);
    let digit_run = seg.split(|c: char| !c.is_ascii_digit()).map(str::len).max().unwrap_or(0);
    hex_run >= 12 || digit_run >= 6 || (seg.len() == 36 && seg.matches('-').count() == 4)
}

/// Generalize a pattern so it says nothing about the person who wrote it.
pub fn generalize(pattern: &str, home: &Path, user: &str) -> String {
    let home_s = home.to_string_lossy().trim_end_matches('/').to_string();
    let mut p = pattern.trim().to_string();
    if let Some(rest) = p.strip_prefix(&home_s) {
        if rest.is_empty() || rest.starts_with('/') {
            p = format!("~{}", rest);
        }
    }
    // Someone else's home (/Users/<x>/…) → ~.
    if let Some(rest) = p.strip_prefix("/Users/") {
        p = match rest.split_once('/') {
            Some((_, r)) => format!("~/{}", r),
            None => "~".into(),
        };
    }
    let (root, rest) = match p.strip_prefix("~/") {
        Some(r) => ("~", r.to_string()),
        None => ("", p.trim_start_matches('/').to_string()),
    };
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let n = segs.len();
    let in_home = root == "~";
    // Areas where folder names are vendor/app names, not the person's own layout.
    let trusted = if in_home {
        segs.first() == Some(&"Library")
    } else {
        matches!(segs.first(), Some(&"Library") | Some(&"Applications") | Some(&"System"))
            || segs.starts_with(&["usr", "local"])
            || segs.starts_with(&["opt", "homebrew"])
    };
    // Outside home the first part is a system root name (Volumes, opt, var …): keep it.
    let base = if in_home { 0 } else { 1 };
    let mut out: Vec<String> = vec![];
    let mut collapsing = false; // inside a run of personal folders → one `**`
    for (i, s) in segs.iter().enumerate() {
        if i < base {
            out.push(s.to_string());
            continue;
        }
        let last = i + 1 == n;
        let glob = s.contains('*') || s.contains('?');
        let generic = if looks_personal(s, user) {
            false
        } else {
            glob || last || s.starts_with('.') || trusted || GENERIC.contains(s)
        };
        // Outside trusted areas, folders before the first generic part are the person's own
        // layout (Documents/Clients/Acme/…, Volumes/Work/…): collapse them.
        if !generic && !trusted && (out.len() == base || collapsing) {
            if !collapsing {
                out.push("**".into());
                collapsing = true;
            }
            continue;
        }
        collapsing = false;
        out.push(if generic { s.to_string() } else { "*".into() });
    }
    let body = out.join("/");
    if in_home {
        format!("~/{}", body)
    } else {
        format!("/{}", body)
    }
}

/// Remove paths, home, user name and emails from free text.
pub fn scrub(text: &str, home: &Path, user: &str, max: usize) -> String {
    let home_s = home.to_string_lossy().trim_end_matches('/').to_string();
    let mut t = text.replace(&home_s, "~");
    let mut words: Vec<String> = vec![];
    for w in t.split(' ') {
        let lw = w.to_lowercase();
        if w.contains('@') || (user.len() >= 3 && lw.contains(&user.to_lowercase())) {
            words.push("*".into());
        } else if w.starts_with("/Users/") || w.starts_with("~/") || w.starts_with("/Volumes/") {
            words.push(generalize(w.trim_end_matches(['.', ',', ')', ';']), home, user));
        } else {
            words.push(w.to_string());
        }
    }
    t = words.join(" ");
    t.chars().take(max).collect::<String>().trim().to_string()
}

pub fn anonymize(rule: &LearnedRule, home: &Path, user: &str, bytes: Option<u64>) -> Submission {
    Submission {
        schema_version: 1,
        pattern: generalize(&rule.pattern, home, user),
        category: rule.category,
        title: scrub(&rule.title, home, user, 80),
        why: scrub(&rule.why, home, user, 300),
        regrows: rule.regrows,
        safety: rule.safety,
        size: bytes.map(size_bucket),
        disko: env!("CARGO_PKG_VERSION").into(),
    }
}

fn outbox_dir(dir: &Path) -> PathBuf {
    dir.join("outbox")
}

fn outbox_file(dir: &Path, generalized: &str) -> PathBuf {
    outbox_dir(dir).join(format!("{}.json", short_hash(generalized)))
}

/// Queue a submission (one per generalized pattern; a newer one replaces the older).
pub fn enqueue(dir: &Path, s: &Submission) -> Result<PathBuf, String> {
    std::fs::create_dir_all(outbox_dir(dir)).map_err(|e| e.to_string())?;
    let f = outbox_file(dir, &s.pattern);
    let json = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    crate::store::write_atomic(&f, &json)?;
    Ok(f)
}

/// Drop a queued submission (rule unshared or removed before it was sent).
pub fn dequeue(dir: &Path, generalized: &str) {
    let _ = std::fs::remove_file(outbox_file(dir, generalized));
}

/// Queued submissions with their files.
pub fn outbox(dir: &Path) -> Vec<(PathBuf, Submission)> {
    let Ok(rd) = std::fs::read_dir(outbox_dir(dir)) else { return vec![] };
    let mut v: Vec<(PathBuf, Submission)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).map(|s| (p, s)))
        .collect();
    v.sort_by(|a, b| a.1.pattern.cmp(&b.1.pattern));
    v
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct SubmitReport {
    pub sent: usize,
    pub queued: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Endpoint for submissions: `DISKO_COMMUNITY_URL`, else the configured one.
pub fn endpoint(configured: Option<&str>) -> Option<String> {
    std::env::var("DISKO_COMMUNITY_URL")
        .ok()
        .or_else(|| configured.map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("https://") || s.starts_with("http://127.0.0.1") || s.starts_with("http://localhost"))
}

/// POST JSON with `curl`.
pub fn http_post_json(url: &str, body: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new("/usr/bin/curl")
        .args(["-fsS", "--max-time", "20", "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-"])
        .arg(url)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {}", e))?;
    child.stdin.take().ok_or("curl stdin")?.write_all(body).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Send queued submissions to `endpoint` with `post`; sent ones leave the outbox. Without an
/// endpoint everything stays queued.
pub fn submit_with(dir: &Path, endpoint: Option<&str>, post: impl Fn(&str, &[u8]) -> Result<(), String>) -> SubmitReport {
    let items = outbox(dir);
    let mut r = SubmitReport::default();
    let Some(url) = endpoint else {
        r.queued = items.len();
        return r;
    };
    for (path, s) in items {
        let body = serde_json::to_vec(&s).unwrap_or_default();
        match post(url, &body) {
            Ok(()) => {
                let _ = std::fs::remove_file(&path);
                r.sent += 1;
            }
            Err(e) => {
                r.queued += 1;
                r.errors.push(e);
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use disko_core::Source;

    #[test]
    fn generalizes_personal_paths() {
        let h = Path::new("/Users/jdoe");
        let g = |p: &str| generalize(p, h, "jdoe");
        assert_eq!(g("/Users/jdoe/Documents/Clients/AcmeCorp/app/.dart_tool"), "~/**/.dart_tool");
        assert_eq!(g("~/Documents/Clients/AcmeCorp/app/build/intermediates"), "~/**/build/intermediates");
        assert_eq!(g("/Users/jdoe/.gradle/caches"), "~/.gradle/caches");
        assert_eq!(g("~/Library/Caches/com.foo.Bar"), "~/Library/Caches/com.foo.Bar");
        assert_eq!(
            g("~/Library/Application Support/Code/User/workspaceStorage/0a1b2c3d4e5f6a7b8c9d"),
            "~/Library/Application Support/Code/User/workspaceStorage/*"
        );
        assert_eq!(g("~/Library/Application Support/jdoe-tool/cache"), "~/Library/Application Support/*/cache");
        assert_eq!(g("/Users/someoneelse/.cache/thing"), "~/.cache/thing");
        assert_eq!(g("/Library/Caches/com.vendor.x"), "/Library/Caches/com.vendor.x");
        assert_eq!(g("~/Projects/*/node_modules/.cache"), "~/**/*/node_modules/.cache");
        assert_eq!(g("/Volumes/ClientDisk/AcmeCorp/web/.turbo"), "/Volumes/**/.turbo");
        assert_eq!(g("/var/folders/x1/abc_def/T/acme/ws/*/.turbo"), "/var/**/*/.turbo");
        assert_eq!(g("/opt/homebrew/var/cache/thing"), "/opt/homebrew/var/cache/thing");
    }

    #[test]
    fn scrubs_and_queues() {
        let h = Path::new("/Users/jdoe");
        let rule = LearnedRule {
            id: "x".into(),
            pattern: "/Users/jdoe/work/acme/.turbo".into(),
            category: Category::Build,
            title: "Turbo cache for jdoe".into(),
            why: "Found 3 GB in /Users/jdoe/work/acme/.turbo, mail jdoe@acme.com".into(),
            regrows: true,
            safety: Safety::Safe,
            source: Source::Agent,
            added: 5,
            shared: true,
        };
        let s = anonymize(&rule, h, "jdoe", Some(3_000_000_000));
        assert_eq!(s.pattern, "~/**/.turbo");
        assert!(!s.title.contains("jdoe"));
        assert!(!s.why.contains("jdoe") && !s.why.contains("acme/"), "{}", s.why);
        assert_eq!(s.size.as_deref(), Some("1–10 GB"));
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("jdoe") && !json.contains("Users"), "{}", json);

        let d = crate::testutil::tempdir("share");
        enqueue(&d, &s).unwrap();
        enqueue(&d, &s).unwrap(); // dedup
        assert_eq!(outbox(&d).len(), 1);
        let r = submit_with(&d, None, |_, _| panic!("no endpoint, no network"));
        assert_eq!((r.sent, r.queued), (0, 1));
        let r = submit_with(&d, Some("https://x"), |_, _| Err("down".into()));
        assert_eq!((r.sent, r.queued), (0, 1));
        let r = submit_with(&d, Some("https://x"), |url, body| {
            assert_eq!(url, "https://x");
            assert!(serde_json::from_slice::<Submission>(body).is_ok());
            Ok(())
        });
        assert_eq!((r.sent, r.queued), (1, 0));
        assert!(outbox(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
