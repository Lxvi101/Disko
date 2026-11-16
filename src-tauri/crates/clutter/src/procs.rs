//! What is running right now: executables and working directories. Used to mark findings `busy`
//! and to recheck right before cleaning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    /// Executable path as `ps` reports it (usually absolute).
    pub exe: String,
    /// Executable name.
    pub name: String,
    /// Display name of the outermost app bundle the process (or an ancestor) runs from.
    pub app: Option<String>,
    pub cwd: Option<PathBuf>,
}

/// A point-in-time process snapshot. `complete` is false when `ps` failed, in which case callers
/// must treat everything as possibly in use.
#[derive(Debug, Clone, Default)]
pub struct Procs {
    pub list: Vec<Proc>,
    pub complete: bool,
}

/// "/Applications/Visual Studio Code.app/Contents/…" → "Visual Studio Code".
pub fn app_of(exe: &str) -> Option<String> {
    let end = exe.find(".app/")?;
    let start = exe[..end].rfind('/').map(|i| i + 1).unwrap_or(0);
    Some(exe[start..end].to_string()).filter(|s| !s.is_empty())
}

impl Procs {
    /// Take a snapshot with one `ps` and one `lsof` limited to working directories.
    pub fn snapshot() -> Procs {
        let ps = Command::new("/bin/ps").args(["-axo", "pid=,ppid=,comm=", "-ww"]).output();
        let Ok(ps) = ps else { return Procs::default() };
        if !ps.status.success() {
            return Procs::default();
        }
        let text = String::from_utf8_lossy(&ps.stdout).to_string();
        let mut table: HashMap<u32, (u32, String)> = HashMap::new();
        for line in text.lines() {
            let line = line.trim_start();
            let Some((pid, rest)) = line.split_once(char::is_whitespace) else { continue };
            let rest = rest.trim_start();
            let Some((ppid, comm)) = rest.split_once(char::is_whitespace) else { continue };
            if let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) {
                table.insert(pid, (ppid, comm.trim().to_string()));
            }
        }
        let app_for = |mut pid: u32| -> Option<String> {
            for _ in 0..24 {
                let (ppid, comm) = table.get(&pid)?;
                if let Some(app) = app_of(comm) {
                    return Some(app);
                }
                if *ppid <= 1 {
                    return None;
                }
                pid = *ppid;
            }
            None
        };
        let mut cwds: HashMap<u32, PathBuf> = HashMap::new();
        if let Ok(out) = Command::new("/usr/sbin/lsof").args(["-nP", "-w", "-d", "cwd", "-F", "pn"]).output() {
            let mut pid = 0u32;
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some(p) = line.strip_prefix('p') {
                    pid = p.parse().unwrap_or(0);
                } else if let Some(n) = line.strip_prefix('n') {
                    if n != "/" && pid != 0 {
                        cwds.insert(pid, PathBuf::from(n));
                    }
                }
            }
        }
        let list = table
            .iter()
            .map(|(pid, (_, comm))| Proc {
                pid: *pid,
                exe: comm.clone(),
                name: comm.rsplit('/').next().unwrap_or(comm).to_string(),
                app: app_for(*pid),
                cwd: cwds.get(pid).cloned(),
            })
            .collect();
        Procs { list, complete: true }
    }

    /// A process with this executable name (exact, or a prefix for names longer than 4).
    pub fn named(&self, names: &[&str]) -> Option<&Proc> {
        self.list.iter().find(|p| names.iter().any(|n| p.name == *n || (n.len() > 4 && p.name.starts_with(n) && p.name.len() <= n.len() + 12)))
    }

    /// A process running from inside this folder (its executable lives there).
    pub fn exe_inside(&self, dir: &Path) -> Option<&Proc> {
        let d = format!("{}/", dir.display());
        self.list.iter().find(|p| p.exe.starts_with(&d))
    }

    /// A process whose working directory is inside this folder.
    pub fn cwd_inside(&self, dir: &Path) -> Option<&Proc> {
        self.list.iter().find(|p| p.cwd.as_ref().is_some_and(|c| c.starts_with(dir)))
    }

    /// One of `tools` working inside `project` (a terminal sitting there doesn't count).
    pub fn tool_in(&self, project: &Path, tools: &[&str]) -> Option<&Proc> {
        self.list.iter().find(|p| p.cwd.as_ref().is_some_and(|c| c.starts_with(project)) && tools.iter().any(|t| p.name == *t || (t.len() > 4 && p.name.starts_with(t))))
    }

    /// The app bundle at `app_path` (e.g. "/Applications/Slack.app") is running.
    pub fn app_running(&self, app_path: &Path) -> Option<&Proc> {
        let d = format!("{}/", app_path.display());
        self.list.iter().find(|p| p.exe.starts_with(&d))
    }

    /// An app with this display name is running ("Xcode", "Slack").
    pub fn app_named(&self, name: &str) -> Option<&Proc> {
        let needle = format!("/{}.app/", name);
        self.list.iter().find(|p| p.exe.contains(&needle))
    }
}

/// "Xcode is running" / "cargo is running in this project".
pub fn who(p: &Proc) -> String {
    match &p.app {
        // A tool an app started: "rust-analyzer (T3 Code)".
        Some(app) if !p.exe.contains(&format!("/{}.app/", app)) => format!("{} ({})", p.name, app),
        Some(app) => app.clone(),
        None => p.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, exe: &str, cwd: Option<&str>) -> Proc {
        Proc { pid: 1, exe: exe.into(), name: name.into(), app: app_of(exe), cwd: cwd.map(PathBuf::from) }
    }

    #[test]
    fn app_names_come_from_the_outermost_bundle() {
        assert_eq!(app_of("/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)").as_deref(), Some("Visual Studio Code"));
        assert_eq!(app_of("/usr/bin/zsh"), None);
    }

    #[test]
    fn tools_hold_projects_but_terminals_do_not() {
        let procs = Procs { list: vec![p("zsh", "/bin/zsh", Some("/p/app")), p("cargo", "/x/cargo", Some("/p/other"))], complete: true };
        assert!(procs.tool_in(Path::new("/p/app"), &["cargo"]).is_none());
        assert!(procs.tool_in(Path::new("/p/other"), &["cargo"]).is_some());
        assert!(procs.cwd_inside(Path::new("/p/app")).is_some());
        assert!(procs.cwd_inside(Path::new("/p/ap")).is_none());
        let apps = Procs { list: vec![p("Xcode", "/Applications/Xcode.app/Contents/MacOS/Xcode", None)], complete: true };
        assert!(apps.app_named("Xcode").is_some());
        assert!(apps.app_running(Path::new("/Applications/Xcode.app")).is_some());
        assert!(apps.app_running(Path::new("/Applications/Xco")).is_none());
    }
}
