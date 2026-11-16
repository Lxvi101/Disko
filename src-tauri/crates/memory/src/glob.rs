//! Tiny path globs: `~/` or `/` rooted, `*` and `?` within one name, `**` for any number of
//! folders. No braces or classes; that keeps matching fast and the rules easy to review.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seg {
    Lit(String),
    Wild(Vec<char>),
    Any, // **
}

impl Seg {
    pub(crate) fn matches(&self, name: &str) -> bool {
        match self {
            Seg::Lit(l) => l == name,
            Seg::Wild(p) => wild(p, &name.chars().collect::<Vec<_>>()),
            Seg::Any => true,
        }
    }
    pub(crate) fn is_glob(&self) -> bool {
        !matches!(self, Seg::Lit(_))
    }
    /// `*`, `**`, `?*` … : matches any name.
    pub(crate) fn is_pure_wildcard(&self) -> bool {
        match self {
            Seg::Lit(_) => false,
            Seg::Any => true,
            Seg::Wild(p) => p.iter().all(|c| *c == '*' || *c == '?'),
        }
    }
}

/// `*` / `?` match on chars (iterative, linear-ish).
fn wild(p: &[char], s: &[char]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A compiled pattern. `~` is expanded with the home given to [`Glob::new`].
#[derive(Debug, Clone)]
pub struct Glob {
    pub(crate) segs: Vec<Seg>,
    /// Leading literal part, as an absolute path ("/Users/me/Library/Caches").
    pub(crate) literal: PathBuf,
}

pub(crate) fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

impl Glob {
    pub fn new(pattern: &str, home: &Path) -> Result<Glob, String> {
        let p = pattern.trim();
        let abs = if p == "~" {
            home.to_string_lossy().to_string()
        } else if let Some(rest) = p.strip_prefix("~/") {
            format!("{}/{}", home.to_string_lossy().trim_end_matches('/'), rest)
        } else if p.starts_with('/') {
            p.to_string()
        } else {
            return Err(format!("`{}` must be an absolute path or start with ~/", pattern));
        };
        if abs.contains('\0') || abs.contains('\n') {
            return Err("Patterns can't contain control characters.".into());
        }
        let mut segs = vec![];
        for s in segments(&abs) {
            if s == "." || s == ".." {
                return Err("Patterns can't contain `.` or `..`.".into());
            }
            segs.push(if s == "**" {
                Seg::Any
            } else if s.contains('*') || s.contains('?') {
                if s.contains("**") {
                    return Err("`**` must be a whole path part, like ~/**/node_modules.".into());
                }
                Seg::Wild(s.chars().collect())
            } else {
                Seg::Lit(s.to_string())
            });
        }
        let mut literal = PathBuf::from("/");
        for s in &segs {
            match s {
                Seg::Lit(l) => literal.push(l),
                _ => break,
            }
        }
        Ok(Glob { segs, literal })
    }

    pub fn has_globstar(&self) -> bool {
        self.segs.contains(&Seg::Any)
    }

    pub fn is_literal(&self) -> bool {
        !self.segs.iter().any(Seg::is_glob)
    }

    /// The whole path matches.
    pub fn matches(&self, path: &str) -> bool {
        full(&self.segs, &segments(path))
    }

    /// Cheap pre-check on a file or folder name: could a path ending in `name` match?
    pub fn name_may_match(&self, name: &str) -> bool {
        match self.segs.last() {
            None => false,
            Some(s) => s.matches(name),
        }
    }

    /// The path, or one of its parent folders, matches.
    pub fn matches_or_inside(&self, path: &str) -> bool {
        if self.is_literal() {
            let lit = self.literal.to_string_lossy();
            let p = path.trim_end_matches('/');
            return p == lit || (p.starts_with(lit.as_ref()) && p.as_bytes().get(lit.len()) == Some(&b'/'));
        }
        let s = segments(path);
        (0..=s.len()).any(|k| full(&self.segs, &s[..k]))
    }

    /// Something strictly below `path` could match (removing `path` would remove a match).
    /// Exact for patterns without `**`; with `**` only the literal prefix is considered, so
    /// `~/**/x` doesn't claim every folder.
    pub fn may_match_below(&self, path: &str) -> bool {
        let s = segments(path);
        if self.has_globstar() {
            let lit = segments_of(&self.literal);
            return lit.len() > s.len() && lit[..s.len()] == s[..];
        }
        self.segs.len() > s.len() && self.segs.iter().zip(&s).all(|(p, n)| p.matches(n))
    }

    /// Existing paths matching a pattern without `**` (symlinked folders aren't followed past the
    /// literal prefix). At most `limit` results.
    pub fn expand(&self, limit: usize) -> Vec<PathBuf> {
        let mut out = vec![];
        if self.has_globstar() {
            return out;
        }
        let lit_n = segments_of(&self.literal).len();
        if std::fs::symlink_metadata(&self.literal).is_err() {
            return out;
        }
        expand_from(&self.literal, &self.segs[lit_n..], limit, &mut out);
        out
    }
}

fn segments_of(p: &Path) -> Vec<String> {
    p.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect()
}

fn expand_from(dir: &Path, rest: &[Seg], limit: usize, out: &mut Vec<PathBuf>) {
    if out.len() >= limit {
        return;
    }
    let Some((first, tail)) = rest.split_first() else {
        out.push(dir.to_path_buf());
        return;
    };
    // Descend only through real folders (no symlinks) below the literal prefix.
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => {}
        _ => return,
    }
    match first {
        Seg::Lit(l) => {
            let p = dir.join(l);
            if std::fs::symlink_metadata(&p).is_ok() {
                expand_from(&p, tail, limit, out);
            }
        }
        _ => {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            let mut names: Vec<String> =
                rd.flatten().take(20_000).map(|e| e.file_name().to_string_lossy().to_string()).collect();
            names.sort();
            for n in names {
                if first.matches(&n) {
                    expand_from(&dir.join(&n), tail, limit, out);
                    if out.len() >= limit {
                        return;
                    }
                }
            }
        }
    }
}

fn full(p: &[Seg], s: &[&str]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((Seg::Any, rest)) => (0..=s.len()).any(|k| full(rest, &s[k..])),
        Some((seg, rest)) => !s.is_empty() && seg.matches(s[0]) && full(rest, &s[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(p: &str) -> Glob {
        Glob::new(p, Path::new("/Users/me")).unwrap()
    }

    #[test]
    fn matching() {
        assert!(g("~/.gradle/caches").matches("/Users/me/.gradle/caches"));
        assert!(!g("~/.gradle/caches").matches("/Users/me/.gradle"));
        assert!(g("~/Library/Caches/com.*.foo").matches("/Users/me/Library/Caches/com.x.foo"));
        assert!(g("~/**/.dart_tool").matches("/Users/me/a/b/.dart_tool"));
        assert!(g("~/**/.dart_tool").matches("/Users/me/.dart_tool"));
        assert!(!g("~/**/.dart_tool").matches("/Users/me/a/.dart_tool/x"));
        assert!(g("~/a/?.log").matches("/Users/me/a/1.log"));
        assert!(!g("~/a/?.log").matches("/Users/me/a/12.log"));
        assert!(g("~/**/*.tmp").matches("/Users/me/a/b/c.tmp"));
    }

    #[test]
    fn inside_and_below() {
        let k = g("~/Projects/keep");
        assert!(k.matches_or_inside("/Users/me/Projects/keep"));
        assert!(k.matches_or_inside("/Users/me/Projects/keep/node_modules"));
        assert!(!k.matches_or_inside("/Users/me/Projects/keeper"));
        assert!(k.may_match_below("/Users/me/Projects"));
        assert!(!k.may_match_below("/Users/me/Projects/keep"));
        let w = g("~/Projects/*/data");
        assert!(w.matches_or_inside("/Users/me/Projects/x/data/y"));
        assert!(w.may_match_below("/Users/me/Projects/x"));
        assert!(!w.may_match_below("/Users/me/Other/x"));
        let s = g("~/**/precious");
        assert!(s.matches_or_inside("/Users/me/a/precious/b"));
        assert!(!s.may_match_below("/Users/me/a"));
    }

    #[test]
    fn rejects_bad() {
        let h = Path::new("/Users/me");
        assert!(Glob::new("relative/x", h).is_err());
        assert!(Glob::new("~/a/../b", h).is_err());
        assert!(Glob::new("~/a**b", h).is_err());
    }

    #[test]
    fn expands_on_disk() {
        let d = crate::testutil::tempdir("glob");
        for p in ["p1/.dart_tool", "p2/.dart_tool", "p3/other"] {
            std::fs::create_dir_all(d.join(p)).unwrap();
        }
        let gl = Glob::new(&format!("{}/*/.dart_tool", d.display()), Path::new("/nonexistent")).unwrap();
        let found = gl.expand(10);
        assert_eq!(found.len(), 2);
        assert_eq!(gl.expand(1).len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
