//! Routines: "clean X every N days". This module only decides *what* a routine would take; the
//! caller cleans it (through the engine / `disko_core::ops`) and reports back with
//! [`crate::Store::record_run`].

use crate::{KeepSet, Routine, RoutineSelect};
use disko_core::{Action, Category, Finding, Safety, Source};

const DAY: i64 = 86_400;

/// What a fresh memory starts with: switched off until the user (or an agent, on their word) turns
/// it on.
pub fn suggested() -> Vec<Routine> {
    vec![Routine {
        id: "build-artifacts".into(),
        title: "Build artifacts".into(),
        select: RoutineSelect { categories: vec![Category::Build], rules: vec![] },
        every_days: 2,
        min_age_days: 2,
        enabled: false,
        review: false,
        last_run: None,
        last_freed: None,
        by_agent: false,
    }]
}

impl Routine {
    /// Next time it should run (unix seconds); None = now (never ran).
    pub fn next_run(&self) -> Option<i64> {
        self.last_run.map(|t| t + self.every_days.max(1) as i64 * DAY)
    }

    pub fn due(&self, now: i64) -> bool {
        self.enabled && self.next_run().is_none_or(|t| now >= t)
    }

    fn selects(&self, f: &Finding) -> bool {
        if self.select.categories.contains(&f.category) {
            return true;
        }
        self.select.rules.iter().any(|r| {
            f.rule == *r
                || f.rule.strip_prefix(r.as_str()).is_some_and(|rest| rest.starts_with('.'))
                || f.rule.strip_prefix("learned.") == Some(r.as_str())
                || f.rule.strip_prefix("community.") == Some(r.as_str())
        })
    }
}

/// Findings from rules an agent or the community pack wrote: never cleaned in the background.
/// Only rules the user made themselves (and Disko's built-in ones) take part in routines.
pub fn from_untrusted_rule(f: &Finding) -> bool {
    matches!(f.source, Source::Agent | Source::Community)
        || f.rule.starts_with("community.")
        || (f.rule.starts_with("learned.") && f.source != Source::User)
}

/// Why a finding isn't taken by a routine, or None if it is.
pub fn reject_reason(f: &Finding, routine: &Routine, now: i64, keep: &KeepSet) -> Option<String> {
    if !routine.selects(f) {
        return Some("not selected by this routine".into());
    }
    if from_untrusted_rule(f) {
        return Some("from an agent's or the community's rule (only cleaned by hand)".into());
    }
    if let Some(k) = keep.finding_kept_by(f) {
        return Some(format!("on the keep list ({})", k.pattern));
    }
    if let Some(b) = &f.busy {
        return Some(format!("in use: {}", b));
    }
    if f.admin {
        return Some("needs an administrator password".into());
    }
    let review_ok = routine.review && matches!(f.action, Action::Trash);
    if f.safety != Safety::Safe && !review_ok {
        return Some("review item (routine only takes safe ones)".into());
    }
    if routine.min_age_days > 0 {
        match f.last_used {
            Some(t) if now - t >= routine.min_age_days as i64 * DAY => {}
            Some(_) => return Some(format!("used in the last {} days", routine.min_age_days)),
            None => return Some("last use unknown".into()),
        }
    }
    None
}

/// The findings a routine would clean right now.
pub fn select(findings: &[Finding], routine: &Routine, now: i64, keep: &KeepSet) -> Vec<Finding> {
    findings.iter().filter(|f| reject_reason(f, routine, now, keep).is_none()).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeepEntry;
    use disko_core::Source;
    use std::path::Path;

    pub fn finding(id: &str, cat: Category, safety: Safety, age_days: i64, path: &str) -> Finding {
        Finding {
            id: id.into(),
            rule: format!("{}.x", serde_json::to_value(cat).unwrap().as_str().unwrap()),
            category: cat,
            title: id.into(),
            detail: None,
            icon: "box".into(),
            paths: vec![path.into()],
            bytes: 100,
            last_used: Some(1_000_000_000 - age_days * DAY),
            safety,
            action: if safety == Safety::Safe { Action::Delete } else { Action::Trash },
            regrows: true,
            why: "".into(),
            busy: None,
            admin: false,
            source: Source::Builtin,
        }
    }

    #[test]
    fn build_routine() {
        let now = 1_000_000_000;
        let mut r = suggested().remove(0);
        assert!(!r.due(now));
        r.enabled = true;
        assert!(r.due(now));
        r.last_run = Some(now - DAY);
        assert!(!r.due(now));
        r.last_run = Some(now - 2 * DAY);
        assert!(r.due(now));

        let keep = KeepSet::new(
            &[KeepEntry { id: "k".into(), pattern: "/Users/me/keep/target".into(), note: None, source: Source::User, added: 0 }],
            Path::new("/Users/me"),
        );
        let mut busy = finding("busy", Category::Build, Safety::Safe, 5, "/Users/me/b/target");
        busy.busy = Some("cargo is running".into());
        let fs = vec![
            finding("old", Category::Build, Safety::Safe, 3, "/Users/me/a/target"),
            finding("fresh", Category::Build, Safety::Safe, 1, "/Users/me/c/target"),
            finding("kept", Category::Build, Safety::Safe, 9, "/Users/me/keep/target"),
            finding("review", Category::Build, Safety::Review, 9, "/Users/me/d/target"),
            finding("cache", Category::Cache, Safety::Safe, 9, "/Users/me/Library/Caches/x"),
            busy,
        ];
        let got: Vec<String> = select(&fs, &r, now, &keep).into_iter().map(|f| f.id).collect();
        assert_eq!(got, vec!["old"]);

        r.review = true;
        let got: Vec<String> = select(&fs, &r, now, &keep).into_iter().map(|f| f.id).collect();
        assert_eq!(got, vec!["old", "review"]);

        let by_rule = Routine { select: RoutineSelect { categories: vec![], rules: vec!["cache".into()] }, review: false, ..r };
        let got: Vec<String> = select(&fs, &by_rule, now, &keep).into_iter().map(|f| f.id).collect();
        assert_eq!(got, vec!["cache"]);
    }

    #[test]
    fn agent_and_community_rules_never_run_in_routines() {
        let now = 1_000_000_000;
        let keep = KeepSet::default();
        let learned = |id: &str, rule: &str, src: Source, safety: Safety| Finding {
            rule: rule.into(),
            source: src,
            ..finding(id, Category::Build, safety, 9, &format!("/Users/me/{}/target", id))
        };
        let fs = vec![
            learned("user", "learned.mine", Source::User, Safety::Safe),
            learned("agent", "learned.agents", Source::Agent, Safety::Safe),
            learned("community", "community.dart-tool", Source::Community, Safety::Safe),
            // Mislabelled sources don't get through either.
            learned("forged", "community.x", Source::User, Safety::Safe),
            learned("forged2", "learned.y", Source::Builtin, Safety::Safe),
            learned("agent-review", "learned.z", Source::Agent, Safety::Review),
        ];
        let mut r = suggested().remove(0);
        r.enabled = true;
        r.review = true;
        let got: Vec<String> = select(&fs, &r, now, &keep).into_iter().map(|f| f.id).collect();
        assert_eq!(got, vec!["user"]);
        let by_rule = Routine {
            select: RoutineSelect { categories: vec![], rules: vec!["agents".into(), "dart-tool".into(), "mine".into()] },
            ..r
        };
        let got: Vec<String> = select(&fs, &by_rule, now, &keep).into_iter().map(|f| f.id).collect();
        assert_eq!(got, vec!["user"]);
        assert!(reject_reason(&fs[1], &by_rule, now, &keep).unwrap().contains("agent"));
    }
}
