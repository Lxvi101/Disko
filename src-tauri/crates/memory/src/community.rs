//! The community rule pack: generic clutter sources contributed by Disko users (anonymized,
//! reviewed and merged in the GitHub repo under `rules/community.json`). Disko ships a copy and
//! fetches the latest at most once a day. Rules are data only: a pack can never carry commands,
//! every rule passes the same validation as user and agent rules, and community rules are always
//! `review` (their findings go to the Trash; they never take part in routines).

use crate::rules::{normalize_rule, validate_rule};
use crate::LearnedRule;
use disko_core::Source;
use serde_json::Value;
use std::path::Path;

pub const PACK_URL: &str = "https://raw.githubusercontent.com/Lxvi101/Disko/main/rules/community.json";
pub const PACK_SCHEMA: u64 = 1;
/// The copy bundled at build time.
pub const BUNDLED: &str = include_str!("../../../../rules/community.json");
const CACHE_FILE: &str = "community.json";
const MAX_BYTES: usize = 1 << 20;
const MAX_RULES: usize = 2000;
/// Keys that would make a rule executable; a rule carrying any of them is dropped.
const FORBIDDEN: [&str; 9] = ["action", "command", "argv", "cmd", "script", "exec", "run", "shell", "program"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pack {
    /// "YYYY-MM-DD" of the pack, used to prefer the newer of bundled and fetched.
    pub updated: String,
    pub rules: Vec<LearnedRule>,
    /// Rules that failed validation and were left out.
    pub dropped: usize,
}

/// Parse and validate a pack. Fails only if the whole file is unusable; bad rules are dropped.
pub fn parse_pack(bytes: &[u8]) -> Result<Pack, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Community pack too large.".into());
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("Community pack isn't JSON: {}", e))?;
    let ver = v.get("schema_version").and_then(Value::as_u64).ok_or("Community pack has no schema_version.")?;
    if ver != PACK_SCHEMA {
        return Err(format!("Community pack schema {} isn't supported (want {}).", ver, PACK_SCHEMA));
    }
    let list = v.get("rules").and_then(Value::as_array).ok_or("Community pack has no rules list.")?;
    let home = disko_core::paths::home();
    let mut pack = Pack { updated: v.get("updated").and_then(Value::as_str).unwrap_or("").to_string(), ..Default::default() };
    for item in list.iter().take(MAX_RULES) {
        let Some(obj) = item.as_object() else {
            pack.dropped += 1;
            continue;
        };
        if FORBIDDEN.iter().any(|k| obj.contains_key(*k)) {
            pack.dropped += 1;
            continue;
        }
        let Ok(mut r) = serde_json::from_value::<LearnedRule>(item.clone()) else {
            pack.dropped += 1;
            continue;
        };
        r.source = Source::Community;
        r.shared = false;
        r.added = 0;
        normalize_rule(&mut r);
        if r.id.is_empty() || validate_rule(&r, &home).is_err() || pack.rules.iter().any(|x| x.id == r.id) {
            pack.dropped += 1;
            continue;
        }
        pack.rules.push(r);
    }
    pack.dropped += list.len().saturating_sub(MAX_RULES);
    Ok(pack)
}

/// The best pack available offline: the fetched copy in `dir` when it's valid and not older than
/// the bundled one, else the bundled one.
pub fn load(dir: &Path) -> Pack {
    let bundled = parse_pack(BUNDLED.as_bytes()).unwrap_or_default();
    match std::fs::read(dir.join(CACHE_FILE)).ok().and_then(|b| parse_pack(&b).ok()) {
        Some(cached) if cached.updated >= bundled.updated && !cached.rules.is_empty() => cached,
        _ => bundled,
    }
}

/// Download with `curl` (always on macOS; no TLS stack in Disko). HTTPS only, size-capped.
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("Only https:// URLs.".into());
    }
    let out = std::process::Command::new("/usr/bin/curl")
        .args(["-fsSL", "--proto", "=https", "--max-time", "20", "--max-filesize"])
        .arg(MAX_BYTES.to_string())
        .arg(url)
        .output()
        .map_err(|e| format!("curl: {}", e))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("Download failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Fetch the latest pack with `get`, validate it, and cache it in `dir`. Returns the pack.
pub fn fetch_with(dir: &Path, url: &str, get: impl Fn(&str) -> Result<Vec<u8>, String>) -> Result<Pack, String> {
    let bytes = get(url)?;
    let pack = parse_pack(&bytes)?;
    if pack.rules.is_empty() {
        return Err("Community pack has no usable rules.".into());
    }
    crate::store::write_atomic(&dir.join(CACHE_FILE), &bytes)?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pack_is_valid() {
        let p = parse_pack(BUNDLED.as_bytes()).unwrap();
        assert!(p.rules.len() >= 5, "bundled pack should have rules");
        assert_eq!(p.dropped, 0, "every bundled rule must validate");
        assert!(p.rules.iter().all(|r| r.source == Source::Community));
    }

    #[test]
    fn community_rules_are_always_review() {
        use disko_core::Safety;
        let p = parse_pack(BUNDLED.as_bytes()).unwrap();
        assert!(p.rules.iter().all(|r| r.safety == Safety::Review));
        // A pack that claims `safe` (or claims to be from the user) still gets review.
        let j = r#"{"schema_version":1,"updated":"2030-01-01","rules":[
            {"id":"evil","pattern":"~/Projects/*/notes","category":"build","title":"Notes","safety":"safe","source":"user"},
            {"id":"thesis","pattern":"~/Documents/Thesis","category":"cache","title":"Cache","safety":"safe"},
            {"id":"git","pattern":"~/Dev/*/.git","category":"build","title":"Git","safety":"safe"},
            {"id":"mail","pattern":"~/Library/Mail/V10","category":"cache","title":"Mail","safety":"safe"},
            {"id":"glob","pattern":"~/**/Thesis","category":"build","title":"Thesis","safety":"safe"},
            {"id":"sys","pattern":"/Library/Caches/com.x","category":"cache","title":"Sys","safety":"safe"}
        ]}"#;
        let p = parse_pack(j.as_bytes()).unwrap();
        assert_eq!(p.dropped, 4, "{:?}", p.rules);
        assert!(p.rules.iter().all(|r| r.safety == Safety::Review && r.source == Source::Community));
    }

    #[test]
    fn rejects_commands_and_bad_rules() {
        let j = r#"{"schema_version":1,"updated":"2030-01-01","rules":[
            {"id":"ok","pattern":"~/.gradle/caches","category":"packages","title":"Gradle","why":"w","regrows":true,"safety":"safe"},
            {"id":"cmd","pattern":"~/.foo/cache","category":"cache","title":"Foo","safety":"safe","command":["rm","-rf","/"]},
            {"id":"broad","pattern":"~/*","category":"cache","title":"All","safety":"safe"},
            {"id":"ok","pattern":"~/.dup/cache","category":"cache","title":"Dup","safety":"safe"},
            {"pattern":"~/.noid/cache","category":"cache","title":"No id","safety":"safe"},
            {"id":"weird","pattern":"~/.x/y","category":"nope","title":"x","safety":"safe"}
        ]}"#;
        let p = parse_pack(j.as_bytes()).unwrap();
        assert_eq!(p.rules.len(), 1);
        assert_eq!(p.dropped, 5);
        assert!(parse_pack(br#"{"schema_version":2,"rules":[]}"#).is_err());
        assert!(parse_pack(b"not json").is_err());
    }

    #[test]
    fn fetch_caches_and_load_prefers_newer() {
        let d = crate::testutil::tempdir("community");
        let newer = r#"{"schema_version":1,"updated":"2999-01-01","rules":[
            {"id":"only","pattern":"~/.gradle/caches","category":"packages","title":"Gradle","safety":"safe"}]}"#;
        let p = fetch_with(&d, PACK_URL, |_| Ok(newer.as_bytes().to_vec())).unwrap();
        assert_eq!(p.rules.len(), 1);
        assert_eq!(load(&d).rules.len(), 1);
        // A broken download leaves the cache alone.
        assert!(fetch_with(&d, PACK_URL, |_| Ok(b"{}".to_vec())).is_err());
        assert_eq!(load(&d).updated, "2999-01-01");
        // An older cached pack loses to the bundled one.
        let older = newer.replace("2999-01-01", "2000-01-01");
        std::fs::write(d.join(CACHE_FILE), older).unwrap();
        assert_eq!(load(&d).updated, parse_pack(BUNDLED.as_bytes()).unwrap().updated);
        let _ = std::fs::remove_dir_all(&d);
        assert!(http_get("http://example.com").is_err());
    }
}
