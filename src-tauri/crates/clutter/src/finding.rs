//! Building `Finding`s consistently.

use crate::util::{needs_admin, Size};
use disko_core::{finding_id, Action, Category, Finding, Safety, Source};
use std::path::Path;

/// A finding with the default action for its safety (Safe → Delete, Review → Trash), an id from
/// rule + paths, and `admin` set when any path is root-owned.
#[allow(clippy::too_many_arguments)]
pub fn make(rule: &str, category: Category, title: impl Into<String>, icon: impl Into<String>, paths: Vec<String>, size: Size, safety: Safety, regrows: bool, why: impl Into<String>) -> Finding {
    let admin = paths.iter().any(|p| needs_admin(Path::new(p)));
    Finding {
        id: finding_id(rule, &paths),
        rule: rule.into(),
        category,
        title: title.into(),
        detail: None,
        icon: icon.into(),
        paths,
        bytes: size.bytes,
        last_used: size.newest,
        safety,
        action: if safety == Safety::Safe { Action::Delete } else { Action::Trash },
        regrows,
        why: why.into(),
        busy: None,
        admin,
        source: Source::Builtin,
    }
}

pub trait FindingExt {
    /// Stable id from a group key instead of the (changing) member paths.
    fn keyed(self, key: &str) -> Self;
    fn detail(self, d: impl Into<String>) -> Self;
    fn command(self, argv: &[&str], label: &str) -> Self;
    fn busy(self, b: Option<String>) -> Self;
}

impl FindingExt for Finding {
    fn keyed(mut self, key: &str) -> Self {
        self.id = finding_id(&self.rule, &[key.to_string()]);
        self
    }
    fn detail(mut self, d: impl Into<String>) -> Self {
        let d = d.into();
        self.detail = if d.is_empty() { None } else { Some(d) };
        self
    }
    fn command(mut self, argv: &[&str], label: &str) -> Self {
        self.action = Action::Command { argv: argv.iter().map(|s| s.to_string()).collect(), label: label.into() };
        self.admin = false;
        self
    }
    fn busy(mut self, b: Option<String>) -> Self {
        self.busy = b;
        self
    }
}
