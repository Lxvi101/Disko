//! Git checks for agent worktrees: what you'd lose by trashing one.

use std::path::{Path, PathBuf};
use std::time::Duration;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut a = vec!["-C", dir.to_str()?];
    a.extend_from_slice(args);
    let (out, _) = crate::util::run_env("/usr/bin/git", &a, &[("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0")], Duration::from_secs(10))?;
    Some(out.trim().to_string())
}

/// Main repository folder of a worktree (parent of the common .git dir).
pub fn main_repo(dir: &Path) -> Option<PathBuf> {
    // Fast path: a linked worktree's `.git` file says "gitdir: <repo>/.git/worktrees/<name>".
    if let Ok(t) = std::fs::read_to_string(dir.join(".git")) {
        if let Some(gd) = t.trim().strip_prefix("gitdir:") {
            let gd = PathBuf::from(gd.trim());
            if let Some(repo_git) = gd.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git")) {
                return repo_git.parent().map(Path::to_path_buf);
            }
        }
    }
    if dir.join(".git").is_dir() {
        return Some(dir.to_path_buf());
    }
    None
}

/// "3 uncommitted changes · 2 commits not pushed" (None when everything is saved elsewhere).
pub fn warning(dir: &Path) -> Option<String> {
    if !crate::util::exists(&dir.join(".git")) {
        return Some("Not a Git checkout: files here exist nowhere else.".into());
    }
    let mut parts = Vec::new();
    match git(dir, &["status", "--porcelain=v1", "--untracked-files=normal"]) {
        Some(st) => {
            let n = st.lines().filter(|l| !l.trim().is_empty()).count();
            if n > 0 {
                parts.push(format!("{} uncommitted change{}", n, if n == 1 { "" } else { "s" }));
            }
        }
        None => parts.push("Git status unavailable".into()),
    }
    // Commits on this HEAD that no remote branch has.
    if let Some(c) = git(dir, &["rev-list", "--count", "HEAD", "--not", "--remotes"]) {
        if let Ok(n) = c.parse::<u64>() {
            if n > 0 {
                let has_remote = git(dir, &["remote"]).is_some_and(|r| !r.is_empty());
                let what = if has_remote { "not pushed" } else { "with no remote" };
                parts.push(format!("{} commit{} {}", n, if n == 1 { "" } else { "s" }, what));
            }
        }
    }
    if let Some(gd) = git(dir, &["rev-parse", "--absolute-git-dir"]) {
        if Path::new(&gd).join("locked").exists() {
            parts.push("worktree is locked".into());
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        assert!(git(dir, args).is_some(), "git {:?}", args);
    }

    #[test]
    fn warns_about_dirty_and_unpushed_worktrees() {
        let home = crate::util::test_dir("git");
        let main = home.join("main");
        let remote = home.join("remote.git");
        let wt = home.join("wt");
        std::fs::create_dir_all(&main).unwrap();
        run(&home, &["init", "-q", "--bare", remote.to_str().unwrap()]);
        run(&main, &["init", "-q", "-b", "main"]);
        let c = ["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-q", "--allow-empty", "-m"];
        run(&main, &[&c[..], &["one"]].concat());
        run(&main, &["remote", "add", "origin", remote.to_str().unwrap()]);
        run(&main, &["push", "-q", "-u", "origin", "main"]);
        run(&main, &["worktree", "add", "-q", "-b", "trial", wt.to_str().unwrap()]);
        assert_eq!(main_repo(&wt).as_deref(), Some(main.as_path()));
        assert_eq!(warning(&wt), None);
        std::fs::write(wt.join("new.txt"), "x").unwrap();
        assert_eq!(warning(&wt).as_deref(), Some("1 uncommitted change"));
        run(&wt, &["add", "new.txt"]);
        run(&wt, &[&c[..], &["two"]].concat());
        assert_eq!(warning(&wt).as_deref(), Some("1 commit not pushed"));
        let plain = home.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(warning(&plain).unwrap().contains("Not a Git checkout"));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
