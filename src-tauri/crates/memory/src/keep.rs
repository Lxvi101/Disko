//! The keep list, compiled for fast checks. The keep list always wins: a finding is kept when one
//! of its paths is a kept path, is inside one, or contains one. Matching ignores case, like APFS:
//! keeping `~/Dev/App` also keeps `~/dev/app/node_modules`.

use crate::{Glob, KeepEntry};
use disko_core::Finding;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct KeepSet {
    entries: Vec<(KeepEntry, Glob)>,
}

impl KeepSet {
    pub fn new(entries: &[KeepEntry], home: &Path) -> KeepSet {
        let home = std::path::PathBuf::from(home.to_string_lossy().to_lowercase());
        KeepSet {
            entries: entries
                .iter()
                .filter_map(|e| Glob::new(&e.pattern.to_lowercase(), &home).ok().map(|g| (e.clone(), g)))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The keep entry protecting `path`, if any.
    pub fn kept_by(&self, path: &str) -> Option<&KeepEntry> {
        if self.entries.is_empty() {
            return None;
        }
        let path = path.to_lowercase();
        self.entries.iter().find(|(_, g)| g.matches_or_inside(&path) || g.may_match_below(&path)).map(|(e, _)| e)
    }

    pub fn is_kept(&self, path: &str) -> bool {
        self.kept_by(path).is_some()
    }

    /// The keep entry protecting any of the finding's paths.
    pub fn finding_kept_by(&self, f: &Finding) -> Option<&KeepEntry> {
        if self.entries.is_empty() {
            return None;
        }
        f.paths.iter().find_map(|p| self.kept_by(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use disko_core::Source;

    fn e(p: &str) -> KeepEntry {
        KeepEntry { id: "k".into(), pattern: p.into(), note: None, source: Source::User, added: 0 }
    }

    #[test]
    fn keep_wins_both_directions() {
        let k = KeepSet::new(&[e("~/Dev/app/node_modules"), e("/Users/me/Models/*.gguf")], Path::new("/Users/me"));
        assert!(k.is_kept("/Users/me/Dev/app/node_modules"));
        assert!(k.is_kept("/Users/me/Dev/app/node_modules/.cache"));
        assert!(k.is_kept("/Users/me/Dev/app")); // contains a kept path
        assert!(!k.is_kept("/Users/me/Dev/other/node_modules"));
        assert!(k.is_kept("/Users/me/Models/llama.gguf"));
        assert!(k.is_kept("/Users/me/Models")); // could contain a match
        assert!(!k.is_kept("/Users/me/Models2"));
    }

    #[test]
    fn keep_ignores_case_like_apfs() {
        let k = KeepSet::new(&[e("~/Dev/App/node_modules"), e("/Users/Me/Models/*.GGUF")], Path::new("/Users/me"));
        assert!(k.is_kept("/Users/me/dev/app/NODE_MODULES"));
        assert!(k.is_kept("/users/ME/Dev/App/node_modules/.cache"));
        assert!(k.is_kept("/Users/me/DEV/app")); // contains a kept path
        assert!(k.is_kept("/Users/me/models/llama.gguf"));
        assert!(!k.is_kept("/Users/me/Dev/other/node_modules"));
        let f = disko_core::Finding {
            id: "f".into(),
            rule: "build.node-modules".into(),
            category: disko_core::Category::Build,
            title: "x".into(),
            detail: None,
            icon: "box".into(),
            paths: vec!["/USERS/me/dev/APP/node_modules".into()],
            bytes: 1,
            last_used: None,
            safety: disko_core::Safety::Safe,
            action: disko_core::Action::Delete,
            regrows: true,
            why: String::new(),
            busy: None,
            admin: false,
            source: Source::Builtin,
        };
        assert_eq!(k.finding_kept_by(&f).map(|x| x.pattern.as_str()), Some("~/Dev/App/node_modules"));
    }
}
