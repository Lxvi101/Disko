//! Learned rules: validation, turning matches into `Finding`s, and a `WalkMatcher` so the engine
//! can recognise `**` rules during its one disk walk.

use crate::glob::Seg;
use crate::{Glob, LearnedRule};
use disko_core::paths::is_protected;
use disko_core::{finding_id, Action, Category, Finding, Matched, Safety, Source, WalkMatcher};
use std::path::{Path, PathBuf};

/// Checks a rule before Disko accepts it (from the user, an agent or the community pack).
/// `home` expands `~` for the protected-path check; pass `disko_core::paths::home()`.
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
    // What follows the root (`~` or `/`).
    let root_n = if p.starts_with('~') { crate::glob::segments(&home.to_string_lossy()).len() } else { 0 };
    let rest = &g.segs[root_n.min(g.segs.len())..];
    if rest.is_empty() {
        return Err("A rule can't target the home folder or the disk root.".into());
    }
    if !rest.iter().any(|s| !s.is_pure_wildcard()) {
        return Err("Too broad: the pattern needs at least one real name (not just `*`/`**`).".into());
    }
    if g.has_globstar() {
        if rest.iter().filter(|s| **s == Seg::Any).count() > 2 {
            return Err("At most two `**` per pattern.".into());
        }
        match rest.last() {
            Some(Seg::Lit(l)) if l.chars().count() >= 3 => {}
            _ => return Err("With `**`, the last part must be an exact name of 3+ characters (like ~/**/.dart_tool).".into()),
        }
    } else if rest.len() == 1 && rest[0].is_glob() {
        return Err("Too broad: no wildcards right below the root.".into());
    }
    // Protected check on a representative path ("x" for wildcards, `**` dropped), in the real home.
    let mut sample = if p.starts_with('~') { disko_core::paths::home() } else { PathBuf::from("/") };
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
        safety: rule.safety,
        action: if rule.safety == Safety::Safe { Action::Delete } else { Action::Trash },
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
            if is_protected(&p) {
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
    tag_base: u32,
}

impl LearnedMatcher {
    pub fn new(rules: &[LearnedRule], home: &Path, tag_base: u32) -> LearnedMatcher {
        LearnedMatcher {
            rules: rules.iter().filter_map(|r| Glob::new(&r.pattern, home).ok().map(|g| (r.clone(), g))).collect(),
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
                if g.matches(full) && !is_protected(path) {
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
        let ok = ["~/.gradle/caches", "~/**/.dart_tool", "~/Library/Caches/com.foo.bar", "~/Library/Caches/com.foo.*", "/Library/Caches/com.foo", "~/Projects/*/build/intermediates"];
        for p in ok {
            assert!(validate_rule(&rule(p), &h).is_ok(), "{} should be ok: {:?}", p, validate_rule(&rule(p), &h));
        }
        let bad = [
            "~", "~/*", "~/**", "/**", "/*", "/", "~/Documents", "~/Library/Caches/*", "~/Library/Application Support/*",
            "~/.ssh/known_hosts", "~/**/*.log", "~/**/*", "~/x/$(rm -rf)", "relative/path", "~/**/a/**/b/**/c",
            "~/Library/Keychains/x", "~/Library/Caches", "/System/Library/Caches/x",
        ];
        for p in bad {
            assert!(validate_rule(&rule(p), &h).is_err(), "{} should be rejected", p);
        }
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
        assert_eq!(f.action, Action::Delete);

        let r2 = LearnedRule { id: "s".into(), pattern: format!("{}/*/.dart_tool", d.display()), ..r };
        let fs = static_findings(&[r2], Path::new("/nope"), 10);
        assert_eq!(fs.len(), 1);
        assert!(fs[0].bytes >= 8192);
        assert!(fs[0].last_used.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }
}
