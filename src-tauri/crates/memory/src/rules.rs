//! Learned rules: validation, turning matches into `Finding`s, and a `WalkMatcher` so the engine
//! can recognise `**` rules during its one disk walk.

use crate::glob::Seg;
use crate::{Glob, LearnedRule};
use disko_core::paths::is_protected;
use disko_core::{finding_id, Action, Category, Finding, Matched, Safety, Source, WalkMatcher};
use std::path::Path;

/// Checks a rule before Disko accepts it (from the user, an agent or the community pack).
/// `home` is the home folder `~` stands for (and the only place rules may point into); pass
/// `disko_core::paths::home()` outside tests.
pub fn validate_rule(rule: &LearnedRule, home: &Path) -> Result<(), String> {
    validate_pattern(&rule.pattern, home)?;
    let t = rule.title.trim();
    if t.is_empty() || t.chars().count() > 80 {
        return Err("A rule needs a short title (1–80 characters).".into());
    }
    if rule.why.chars().count() > 300 {
        return Err("`why` should be one short sentence (≤ 300 characters).".into());
    }
    if rule.id.chars().count() > 64 || rule.id.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')) {
        return Err("Rule ids use a-z, 0-9, `-`, `_`, `.` (≤ 64).".into());
    }
    if rule.category == Category::Forgotten && rule.safety == Safety::Safe {
        return Err("Forgotten personal files are never `safe`; use `review`.".into());
    }
    Ok(())
}

/// Only the user may mark a rule `safe` (permanently deleted, usable by routines). Rules from
/// agents and the community pack are always `review`: their findings go to the Trash.
pub fn effective_safety(rule: &LearnedRule) -> Safety {
    match rule.source {
        Source::User | Source::Builtin => rule.safety,
        _ => Safety::Review,
    }
}

/// Bring a rule in line with what its source may do (see [`effective_safety`]). Applied to every
/// rule Disko reads or accepts: from memory.json, the community pack, the UI and agents.
pub fn normalize_rule(rule: &mut LearnedRule) {
    rule.safety = effective_safety(rule);
}

/// Folder names that `**` rules may end in: well-known, rebuildable build/tool output.
/// Compared case-insensitively. `.wrangler/tmp` is the one two-part entry.
pub const GLOBSTAR_NAMES: [&str; 40] = [
    "node_modules", "target", ".build", "build", "dist", ".next", ".turbo", ".nuxt", ".svelte-kit", ".parcel-cache",
    ".cache", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".gradle", "deriveddata", "pods",
    ".dart_tool", ".angular", ".vite", "coverage", ".tox", ".nox", "zig-cache", ".zig-cache", ".terraform",
    "cmakefiles", "elm-stuff", ".stack-work", "dist-newstyle", "_build", "deps", ".expo", ".wrangler/tmp", "out",
    ".output", ".vercel", ".docusaurus", "storybook-static",
];

/// Credentials and other folders/files below home that no rule may touch, nor any folder that
/// contains them (lowercase, home-relative).
const SECRETS: [&[&str]; 18] = [
    &[".ssh"], &[".aws"], &[".kube"], &[".gnupg"], &[".gpg"], &[".azure"], &[".password-store"], &[".netrc"],
    &[".npmrc"], &[".pypirc"], &[".git-credentials"], &[".config", "gh"], &[".config", "gcloud"],
    &[".docker", "config.json"], &[".cargo", "credentials"], &[".cargo", "credentials.toml"], &["keychains"],
    &[".local", "share", "keyrings"],
];

/// Why a home-relative location is off limits for rules, or None. `n` is the number of parts,
/// `is(i, name)` says whether part `i` is (or certainly matches) the lowercase `name`. With
/// `open_end` the parts are only a prefix (a pattern cut at `**`): then "is an ancestor of a
/// secret" can't be decided here and is left to the match-time check.
fn off_limits(n: usize, is: impl Fn(usize, &str) -> bool, open_end: bool) -> Option<String> {
    if (0..n).any(|i| is(i, ".git")) {
        return Some("Disko never touches a Git repository's `.git` folder.".into());
    }
    if n >= 1 && is(0, "library") {
        let ok = n >= 3 && (is(1, "caches") || is(1, "logs"));
        if !ok {
            return Some(
                "Inside ~/Library, rules may only point into ~/Library/Caches/<app>/… or ~/Library/Logs/<app>/… (app data, mail, messages, photos, containers and keychains are off limits).".into(),
            );
        }
    }
    for s in SECRETS {
        let k = n.min(s.len());
        if !(0..k).all(|i| is(i, s[i])) {
            continue;
        }
        if n >= s.len() || !open_end {
            return Some(format!("~/{} holds credentials; Disko never touches it.", s.join("/")));
        }
    }
    None
}

/// Whether a concrete path found by a rule is off limits (case-insensitive, like APFS). Paths
/// outside `home` are left to `is_protected`.
pub fn path_off_limits(path: &Path, home: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    let h = home.to_string_lossy().to_lowercase();
    let ps = crate::glob::segments(&p);
    let hs = crate::glob::segments(&h);
    if hs.is_empty() || ps.len() < hs.len() || ps[..hs.len()] != hs[..] {
        return ps.contains(&".git");
    }
    let rel = &ps[hs.len()..];
    off_limits(rel.len(), |i, name| rel[i] == name, false).is_some()
}

/// Does a pattern part certainly stand for `name`? Pure wildcards (`*`, `**`) don't count: what
/// they match is checked on the concrete paths ([`path_off_limits`]).
fn seg_is(seg: &Seg, name: &str) -> bool {
    !seg.is_pure_wildcard() && seg.matches(name)
}

/// Home segments of `p` if it lies in `home` (case-insensitive).
fn home_prefix_len(p: &str, home: &Path) -> Option<usize> {
    if p.starts_with("~/") || p == "~" {
        return Some(crate::glob::segments(&home.to_string_lossy()).len());
    }
    let hs: Vec<String> = crate::glob::segments(&home.to_string_lossy()).iter().map(|s| s.to_lowercase()).collect();
    let ps: Vec<String> = crate::glob::segments(p).iter().map(|s| s.to_lowercase()).collect();
    (!hs.is_empty() && ps.len() >= hs.len() && ps[..hs.len()] == hs[..]).then_some(hs.len())
}

/// Pattern checks shared by rules and the anonymizer.
pub fn validate_pattern(pattern: &str, home: &Path) -> Result<(), String> {
    let p = pattern.trim();
    if p.len() > 300 {
        return Err("Pattern too long.".into());
    }
    if p.contains("$(") || p.contains('`') || p.contains(';') && p.contains(' ') || p.contains('|') || p.contains('\n') {
        return Err("Patterns are paths, never commands.".into());
    }
    let g = Glob::new(p, home)?;
    // Rules live in the home folder; system locations belong to Disko's built-in rules.
    let Some(root_n) = home_prefix_len(p, home) else {
        return Err("Rules can only point inside your home folder (~/…).".into());
    };
    let rest = &g.segs[root_n.min(g.segs.len())..];
    if rest.is_empty() {
        return Err("A rule can't target the home folder or the disk root.".into());
    }
    if !rest.iter().any(|s| !s.is_pure_wildcard()) {
        return Err("Too broad: the pattern needs at least one real name (not just `*`/`**`).".into());
    }
    // Case-insensitive checks (APFS): compare lowercase parts.
    let lower: Vec<Seg> = Glob::new(&p.to_lowercase(), home)?.segs[root_n.min(g.segs.len())..].to_vec();
    let cut = lower.iter().position(|s| *s == Seg::Any).unwrap_or(lower.len());
    let any_git = lower.iter().any(|s| seg_is(s, ".git"));
    if any_git {
        return Err("Disko never touches a Git repository's `.git` folder.".into());
    }
    if let Some(why) = off_limits(cut, |i, name| seg_is(&lower[i], name), cut < lower.len()) {
        return Err(why);
    }
    if g.has_globstar() {
        if rest.iter().filter(|s| **s == Seg::Any).count() > 2 {
            return Err("At most two `**` per pattern.".into());
        }
        let known = match (lower.len().checked_sub(2).map(|i| &lower[i]), lower.last()) {
            (Some(Seg::Lit(a)), Some(Seg::Lit(b))) if a == ".wrangler" && b == "tmp" => true,
            (_, Some(Seg::Lit(l))) => GLOBSTAR_NAMES.contains(&l.as_str()),
            _ => false,
        };
        if !known {
            return Err(format!(
                "With `**`, the last part must be a known rebuildable folder ({} …), like ~/**/.dart_tool.",
                GLOBSTAR_NAMES[..8].join(", ")
            ));
        }
    } else if rest.len() == 1 && rest[0].is_glob() {
        return Err("Too broad: no wildcards right below the root.".into());
    }
    // Protected check on a representative path ("x" for wildcards, `**` dropped), in the real home.
    let mut sample = disko_core::paths::home();
    for s in rest {
        match s {
            Seg::Lit(l) => sample.push(l),
            Seg::Wild(_) => sample.push("x"),
            Seg::Any => {}
        }
    }
    if is_protected(&sample) {
        return Err(format!("{} is protected; Disko never removes it.", sample.display()));
    }
    // `…/<protected folder>/*` means "everything in Caches/Application Support/…": too broad.
    if let Some(last) = rest.last() {
        if last.is_pure_wildcard() {
            let parent = sample.parent().map(Path::to_path_buf).unwrap_or_default();
            let parent = if matches!(last, Seg::Wild(_)) { parent } else { sample.clone() };
            if is_protected(&parent) {
                return Err(format!("Too broad: that's everything in {}.", parent.display()));
            }
        }
    }
    Ok(())
}

/// Rule id used on findings: `learned.<id>` or `community.<id>`.
pub fn finding_rule(rule: &LearnedRule) -> String {
    let kind = if rule.source == Source::Community { "community" } else { "learned" };
    format!("{}.{}", kind, rule.id)
}

/// Allocated bytes and newest mtime below a path (no symlink following).
pub fn measure(path: &Path) -> (u64, Option<i64>) {
    use std::os::unix::fs::MetadataExt;
    fn walk(p: &Path, seen: &mut std::collections::HashSet<(u64, u64)>, acc: &mut (u64, Option<i64>), budget: &mut u32) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(m) = std::fs::symlink_metadata(p) else { return };
        if m.nlink() > 1 && !m.is_dir() && !seen.insert((m.dev(), m.ino())) {
            return;
        }
        acc.0 += m.blocks() * 512;
        acc.1 = Some(acc.1.map_or(m.mtime(), |t| t.max(m.mtime())));
        if m.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), seen, acc, budget);
                }
            }
        }
    }
    let mut acc = (0, None);
    walk(path, &mut Default::default(), &mut acc, &mut 2_000_000);
    acc
}

fn icon(c: Category) -> &'static str {
    match c {
        Category::Logs => "log",
        Category::Trash => "trash",
        Category::Containers => "docker",
        Category::Developer => "xcode",
        _ => "box",
    }
}

/// A finding for one path matched by a learned rule.
pub fn to_finding(rule: &LearnedRule, path: &Path, bytes: u64, last_used: Option<i64>) -> Finding {
    let rid = finding_rule(rule);
    let p = path.to_string_lossy().to_string();
    let (title, detail) = if rule.pattern.contains("**") {
        // Many matches (one per project): name the project, the rule goes second.
        let project = path.parent().and_then(|x| x.file_name()).map(|s| s.to_string_lossy().to_string());
        (project.unwrap_or_else(|| rule.title.clone()), Some(rule.title.clone()))
    } else {
        (rule.title.clone(), Some(disko_core::paths::short(&p)))
    };
    Finding {
        id: finding_id(&rid, std::slice::from_ref(&p)),
        rule: rid,
        category: rule.category,
        title,
        detail,
        icon: icon(rule.category).into(),
        paths: vec![p],
        bytes,
        last_used,
        safety: effective_safety(rule),
        // Permanent deletion only for the user's own `safe` rules; everything else goes to the Trash.
        action: if effective_safety(rule) == Safety::Safe { Action::Delete } else { Action::Trash },
        regrows: rule.regrows,
        why: rule.why.clone(),
        busy: None,
        admin: false,
        source: rule.source,
    }
}

/// Findings for rules without `**`, by looking at the disk directly (bounded). `**` rules come
/// from the engine walk through [`LearnedMatcher`].
pub fn static_findings(rules: &[LearnedRule], home: &Path, limit_per_rule: usize) -> Vec<Finding> {
    let mut out = vec![];
    for r in rules {
        let Ok(g) = Glob::new(&r.pattern, home) else { continue };
        for p in g.expand(limit_per_rule) {
            if is_protected(&p) || path_off_limits(&p, home) {
                continue;
            }
            let (b, t) = measure(&p);
            out.push(to_finding(r, &p, b, t));
        }
    }
    out
}

/// Recognises learned rules during the engine's disk walk. Tags are `tag_base + index`.
pub struct LearnedMatcher {
    rules: Vec<(LearnedRule, Glob)>,
    home: std::path::PathBuf,
    tag_base: u32,
}

impl LearnedMatcher {
    pub fn new(rules: &[LearnedRule], home: &Path, tag_base: u32) -> LearnedMatcher {
        LearnedMatcher {
            rules: rules
                .iter()
                .filter_map(|r| {
                    let mut r = r.clone();
                    normalize_rule(&mut r);
                    Glob::new(&r.pattern, home).ok().map(|g| (r, g))
                })
                .collect(),
            home: home.to_path_buf(),
            tag_base,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn hit(&self, path: &Path, name: &str) -> Option<u32> {
        let mut s: Option<String> = None;
        for (i, (_, g)) in self.rules.iter().enumerate() {
            if g.name_may_match(name) {
                let full = s.get_or_insert_with(|| path.to_string_lossy().to_string());
                if g.matches(full) && !is_protected(path) && !path_off_limits(path, &self.home) {
                    return Some(self.tag_base + i as u32);
                }
            }
        }
        None
    }

    pub fn rule(&self, tag: u32) -> Option<&LearnedRule> {
        tag.checked_sub(self.tag_base).and_then(|i| self.rules.get(i as usize)).map(|(r, _)| r)
    }

    /// The finding for something the engine matched with one of our tags.
    pub fn finding(&self, m: &Matched) -> Option<Finding> {
        let r = self.rule(m.tag)?;
        Some(to_finding(r, Path::new(&m.path), m.bytes, m.newest))
    }

    /// Which rule covers `path` (equal or inside a match), for `explain`.
    pub fn covering(&self, path: &str) -> Option<&LearnedRule> {
        self.rules.iter().find(|(_, g)| g.matches_or_inside(path)).map(|(r, _)| r)
    }
}

impl WalkMatcher for LearnedMatcher {
    fn on_dir(&self, path: &Path, name: &str, _siblings: &[&str]) -> Option<u32> {
        self.hit(path, name)
    }
    fn on_file(&self, path: &Path, name: &str, _bytes: u64, _mtime: i64) -> Option<u32> {
        self.hit(path, name)
    }
    fn category(&self, tag: u32) -> Category {
        self.rule(tag).map(|r| r.category).unwrap_or(Category::Cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(p: &str) -> LearnedRule {
        LearnedRule {
            id: "x".into(),
            pattern: p.into(),
            category: Category::Build,
            title: "X".into(),
            why: "w".into(),
            regrows: true,
            safety: Safety::Safe,
            source: Source::Agent,
            added: 0,
            shared: false,
        }
    }

    #[test]
    fn validation() {
        let h = disko_core::paths::home();
        let home_abs = format!("{}/Dev/*/.turbo", h.display());
        let ok = [
            "~/.gradle/caches", "~/**/.dart_tool", "~/**/NODE_MODULES", "~/Dev/**/target", "~/**/.wrangler/tmp",
            "~/Library/Caches/com.foo.bar", "~/Library/Caches/com.foo.*", "~/Library/Logs/com.foo", "~/Library/Caches/Yarn/v6",
            "~/Projects/*/build/intermediates", home_abs.as_str(), "~/.cargo/registry/cache", "~/.docker/buildx/cache",
        ];
        for p in ok {
            assert!(validate_rule(&rule(p), &h).is_ok(), "{} should be ok: {:?}", p, validate_rule(&rule(p), &h));
        }
        let bad = [
            "~", "~/*", "~/**", "/**", "/*", "/", "~/Documents", "~/Library/Caches/*", "~/Library/Application Support/*",
            "~/.ssh/known_hosts", "~/**/*.log", "~/**/*", "~/x/$(rm -rf)", "relative/path", "~/**/a/**/b/**/c",
            "~/Library/Keychains/x", "~/Library/Caches", "/System/Library/Caches/x",
            // Outside home: system locations are built-in rules' business.
            "/Library/Caches/com.foo", "/tmp/x/build", "/opt/homebrew/var/cache", "/Volumes/Work/proj/target",
            // .git, anywhere and in any case.
            "~/Dev/app/.git", "~/Dev/app/.git/objects", "~/Dev/*/.GIT", "~/Dev/app/.Git/lfs/cache",
            // ~/Library outside Caches/<x> and Logs/<x>.
            "~/Library/Application Support/MobileSync/Backup", "~/Library/Group Containers/x.y", "~/Library/Containers/com.x/Data",
            "~/Library/Mail/V10", "~/Library/Messages/Attachments", "~/Library/Safari/x", "~/Library/Calendars/x",
            "~/Library/Application Support/AddressBook", "~/Library/Group Containers/group.com.apple.notes", "~/Library/Photos/Libraries",
            "~/library/application support/foo", "~/LIBRARY/Mail/x", "~/Library/Developer/Xcode/DerivedData", "~/Library/Logs",
            "~/Library/**/target", "~/Library/Caches/**/node_modules", "~/Lib*/Mail/x",
            // Credentials, and folders that contain them.
            "~/.SSH/id_rsa", "~/.aws/credentials", "~/.kube/config", "~/.gnupg/x", "~/.config/gh", "~/.config/gh/hosts.yml",
            "~/.Config/GH", "~/.config", "~/.docker/config.json", "~/.docker", "~/.netrc", "~/.ssh/**/target", "~/.aws/*/x",
            // `**` must end in a known rebuildable folder.
            "~/**/Thesis", "~/**/src", "~/**/Documents", "~/**/tmp", "~/**/.cache2", "~/Dev/**/notes",
        ];
        for p in bad {
            assert!(validate_rule(&rule(p), &h).is_err(), "{} should be rejected", p);
        }
        // The home that `~` means is the one given, also for absolute patterns (case-insensitive).
        let me = Path::new("/Users/me");
        assert!(validate_pattern("/Users/me/Dev/*/target", me).is_ok());
        assert!(validate_pattern("/users/ME/Dev/*/target", me).is_ok());
        assert!(validate_pattern("/Users/other/Dev/*/target", me).is_err());
        let mut r = rule("~/.gradle/caches");
        r.title = "".into();
        assert!(validate_rule(&r, &h).is_err());
    }

    #[test]
    fn matcher_and_findings() {
        let d = crate::testutil::tempdir("rules");
        std::fs::create_dir_all(d.join("proj/.dart_tool/x")).unwrap();
        std::fs::write(d.join("proj/.dart_tool/x/f"), vec![0u8; 8192]).unwrap();
        let mut r = rule(&format!("{}/**/.dart_tool", d.display()));
        r.id = "dart-tool".into();
        let m = LearnedMatcher::new(&[r.clone()], Path::new("/nope"), 100);
        assert_eq!(m.on_dir(&d.join("proj/.dart_tool"), ".dart_tool", &[]), Some(100));
        assert_eq!(m.on_dir(&d.join("proj/src"), "src", &[]), None);
        assert_eq!(m.category(100), Category::Build);
        let f = m.finding(&Matched { tag: 100, path: d.join("proj/.dart_tool").to_string_lossy().into(), bytes: 5, files: 1, newest: Some(1) }).unwrap();
        assert_eq!(f.rule, "learned.dart-tool");
        assert_eq!(f.title, "proj");
        // An agent's rule never deletes permanently, whatever safety it claims.
        assert_eq!(f.safety, Safety::Review);
        assert_eq!(f.action, Action::Trash);
        let user = LearnedMatcher::new(&[LearnedRule { source: Source::User, ..r.clone() }], Path::new("/nope"), 100);
        let f = user.finding(&Matched { tag: 100, path: d.join("proj/.dart_tool").to_string_lossy().into(), bytes: 5, files: 1, newest: Some(1) }).unwrap();
        assert_eq!((f.safety, f.action), (Safety::Safe, Action::Delete));

        let r2 = LearnedRule { id: "s".into(), pattern: format!("{}/*/.dart_tool", d.display()), ..r };
        let fs = static_findings(&[r2], Path::new("/nope"), 10);
        assert_eq!(fs.len(), 1);
        assert!(fs[0].bytes >= 8192);
        assert!(fs[0].last_used.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn safety_by_source() {
        for (src, want) in [(Source::User, Safety::Safe), (Source::Agent, Safety::Review), (Source::Community, Safety::Review)] {
            let mut r = LearnedRule { source: src, ..rule("~/.gradle/caches") };
            assert_eq!(effective_safety(&r), want, "{:?}", src);
            normalize_rule(&mut r);
            assert_eq!(r.safety, want);
            let f = to_finding(&r, Path::new("/Users/me/.gradle/caches"), 1, None);
            assert_eq!(f.action, if want == Safety::Safe { Action::Delete } else { Action::Trash });
        }
    }

    #[test]
    fn match_time_guard_skips_off_limits_paths() {
        let home = Path::new("/Users/me");
        for p in [
            "/Users/me/Dev/app/.git/target", "/Users/me/.ssh/node_modules", "/Users/me/.SSH/x/target", "/Users/me/Library/Mail/x/build",
            "/Users/me/Library/Application Support/x/node_modules", "/Users/me/library/caches", "/Users/me/.config/gh/x/dist",
        ] {
            assert!(path_off_limits(Path::new(p), home), "{} should be off limits", p);
        }
        for p in ["/Users/me/Dev/app/target", "/Users/me/Library/Caches/com.x/build", "/Users/me/.config/x/node_modules", "/tmp/x/target"] {
            assert!(!path_off_limits(Path::new(p), home), "{} should be fine", p);
        }
        // `~/*/target` passes validation; a `.ssh/target` it matches is skipped at match time.
        let r = LearnedRule { id: "t".into(), pattern: "~/**/target".into(), ..rule("~/x") };
        let m = LearnedMatcher::new(&[r], home, 7);
        assert_eq!(m.on_dir(Path::new("/Users/me/Dev/app/target"), "target", &[]), Some(7));
        assert_eq!(m.on_dir(Path::new("/Users/me/.ssh/target"), "target", &[]), None);
        assert_eq!(m.on_dir(Path::new("/Users/me/Dev/app/.git/target"), "target", &[]), None);
    }
}
