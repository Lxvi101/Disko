//! Register Disko's MCP server (`<exe> --mcp`) with coding agents: Claude Code (through its own
//! CLI when available), Codex (`~/.codex/config.toml`) and Cursor (`~/.cursor/mcp.json`). Config
//! files are edited in place, keeping everything else, and written atomically.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SERVER_NAME: &str = "disko";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    ClaudeCode,
    Codex,
    Cursor,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::ClaudeCode, Target::Codex, Target::Cursor];
    pub fn name(self) -> &'static str {
        match self {
            Target::ClaudeCode => "Claude Code",
            Target::Codex => "Codex",
            Target::Cursor => "Cursor",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetInfo {
    pub target: Target,
    pub name: String,
    /// The agent seems to be installed on this Mac.
    pub detected: bool,
    /// Disko is registered with it.
    pub installed: bool,
    /// Config file Disko's entry lives in.
    pub config: String,
}

/// Where to look. `Installer::new()` uses the real home and finds the `claude` CLI.
#[derive(Debug, Clone)]
pub struct Installer {
    pub home: PathBuf,
    /// Path of the `claude` CLI; None = edit `~/.claude.json` directly.
    pub claude_cli: Option<PathBuf>,
}

impl Default for Installer {
    fn default() -> Self {
        Installer::new()
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
    let tmp = dir.join(format!(".{}.disko-tmp-{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    let res = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(|e| format!("Couldn't write {}: {}", path.display(), e))
}

fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(b) if b.iter().all(|c| c.is_ascii_whitespace()) => Ok(json!({})),
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{} isn't valid JSON ({}); not touching it.", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

fn entry(exe: &Path) -> Value {
    json!({ "command": exe.to_string_lossy(), "args": ["--mcp"] })
}

impl Installer {
    pub fn new() -> Installer {
        let home = disko_core::paths::home();
        let mut candidates = vec![
            home.join(".local/bin/claude"),
            home.join(".claude/local/claude"),
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
        ];
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|d| d.join("claude")));
        }
        let claude_cli = candidates.into_iter().find(|p| p.is_file());
        Installer { home, claude_cli }
    }

    pub fn with_home(home: impl Into<PathBuf>, claude_cli: Option<PathBuf>) -> Installer {
        Installer { home: home.into(), claude_cli }
    }

    pub fn config_path(&self, t: Target) -> PathBuf {
        match t {
            Target::ClaudeCode => self.home.join(".claude.json"),
            Target::Codex => self.home.join(".codex/config.toml"),
            Target::Cursor => self.home.join(".cursor/mcp.json"),
        }
    }

    pub fn detected(&self, t: Target) -> bool {
        match t {
            Target::ClaudeCode => self.claude_cli.is_some() || self.home.join(".claude").is_dir() || self.home.join(".claude.json").is_file(),
            Target::Codex => self.home.join(".codex").is_dir(),
            Target::Cursor => self.home.join(".cursor").is_dir() || self.home.join("Applications/Cursor.app").is_dir() || Path::new("/Applications/Cursor.app").is_dir(),
        }
    }

    pub fn targets(&self) -> Vec<TargetInfo> {
        Target::ALL
            .iter()
            .map(|&t| TargetInfo {
                target: t,
                name: t.name().into(),
                detected: self.detected(t),
                installed: self.is_installed(t),
                config: self.config_path(t).to_string_lossy().into(),
            })
            .collect()
    }

    pub fn is_installed(&self, t: Target) -> bool {
        let p = self.config_path(t);
        match t {
            Target::ClaudeCode | Target::Cursor => read_json(&p).ok().is_some_and(|v| v.pointer(&format!("/mcpServers/{}", SERVER_NAME)).is_some()),
            Target::Codex => std::fs::read_to_string(&p)
                .ok()
                .and_then(|s| s.parse::<toml_edit::DocumentMut>().ok())
                .is_some_and(|d| d.get("mcp_servers").and_then(|m| m.get(SERVER_NAME)).is_some()),
        }
    }

    /// Register `<exe> --mcp` as the `disko` MCP server (replacing an older registration).
    pub fn install(&self, t: Target, exe: &Path) -> Result<(), String> {
        if !exe.is_absolute() {
            return Err("The Disko executable path must be absolute.".into());
        }
        match t {
            Target::ClaudeCode => match &self.claude_cli {
                Some(cli) => {
                    let _ = self.claude(cli, &["mcp", "remove", "--scope", "user", SERVER_NAME]);
                    let exe_s = exe.to_string_lossy();
                    self.claude(cli, &["mcp", "add", "--scope", "user", SERVER_NAME, "--", &exe_s, "--mcp"])
                }
                None => self.edit_json(t, |v| {
                    let mut e = entry(exe);
                    e["type"] = json!("stdio");
                    e["env"] = json!({});
                    v["mcpServers"][SERVER_NAME] = e;
                }),
            },
            Target::Cursor => self.edit_json(t, |v| v["mcpServers"][SERVER_NAME] = entry(exe)),
            Target::Codex => self.edit_toml(|doc| {
                use toml_edit::{value, Array, Item, Table};
                if !doc.contains_key("mcp_servers") {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    doc.insert("mcp_servers", Item::Table(t));
                }
                let servers = doc["mcp_servers"].as_table_mut().ok_or("`mcp_servers` in config.toml isn't a table.")?;
                let mut t = Table::new();
                t.insert("command", value(exe.to_string_lossy().as_ref()));
                let mut args = Array::new();
                args.push("--mcp");
                t.insert("args", value(args));
                servers.insert(SERVER_NAME, Item::Table(t));
                Ok(())
            }),
        }
    }

    pub fn uninstall(&self, t: Target) -> Result<(), String> {
        if !self.is_installed(t) {
            return Ok(());
        }
        match t {
            Target::ClaudeCode => match &self.claude_cli {
                Some(cli) => self.claude(cli, &["mcp", "remove", "--scope", "user", SERVER_NAME]),
                None => self.edit_json(t, |v| {
                    if let Some(m) = v.get_mut("mcpServers").and_then(Value::as_object_mut) {
                        m.remove(SERVER_NAME);
                    }
                }),
            },
            Target::Cursor => self.edit_json(t, |v| {
                if let Some(m) = v.get_mut("mcpServers").and_then(Value::as_object_mut) {
                    m.remove(SERVER_NAME);
                }
            }),
            Target::Codex => self.edit_toml(|doc| {
                if let Some(m) = doc.get_mut("mcp_servers").and_then(|m| m.as_table_mut()) {
                    m.remove(SERVER_NAME);
                }
                Ok(())
            }),
        }
    }

    fn claude(&self, cli: &Path, args: &[&str]) -> Result<(), String> {
        let out = std::process::Command::new(cli)
            .args(args)
            .env("HOME", &self.home)
            .output()
            .map_err(|e| format!("{}: {}", cli.display(), e))?;
        if out.status.success() {
            Ok(())
        } else {
            let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if msg.is_empty() {
                msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
            Err(format!("claude {}: {}", args.join(" "), msg))
        }
    }

    fn edit_json(&self, t: Target, f: impl FnOnce(&mut Value)) -> Result<(), String> {
        let p = self.config_path(t);
        let mut v = read_json(&p)?;
        if !v.is_object() {
            return Err(format!("{} isn't a JSON object; not touching it.", p.display()));
        }
        if v.get("mcpServers").is_some_and(|m| !m.is_object()) {
            return Err(format!("`mcpServers` in {} isn't an object; not touching it.", p.display()));
        }
        f(&mut v);
        let mut bytes = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        atomic_write(&p, &bytes)
    }

    fn edit_toml(&self, f: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>) -> Result<(), String> {
        let p = self.config_path(Target::Codex);
        let text = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.to_string()),
        };
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{} isn't valid TOML ({}); not touching it.", p.display(), e))?;
        f(&mut doc)?;
        atomic_write(&p, doc.to_string().as_bytes())
    }
}

/// Agents on this Mac and whether Disko is registered with them.
pub fn install_targets() -> Vec<TargetInfo> {
    Installer::new().targets()
}

pub fn install(t: Target, exe: &Path) -> Result<(), String> {
    Installer::new().install(t, exe)
}

pub fn is_installed(t: Target) -> bool {
    Installer::new().is_installed(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("disko-mcp-install-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn codex_toml_preserves_other_content() {
        let home = tmp("codex");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let original = "# my settings\nmodel = \"gpt-5\"  # keep this comment\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = [\"x\"]\n\n[profiles.fast]\nmodel = \"mini\"\n";
        std::fs::write(home.join(".codex/config.toml"), original).unwrap();
        let i = Installer::with_home(&home, None);
        assert!(i.detected(Target::Codex));
        assert!(!i.is_installed(Target::Codex));
        i.install(Target::Codex, Path::new("/Applications/Disko.app/Contents/MacOS/disko")).unwrap();
        assert!(i.is_installed(Target::Codex));
        let after = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(after.contains("# my settings") && after.contains("# keep this comment") && after.contains("[profiles.fast]"));
        assert!(after.contains("[mcp_servers.other]"));
        assert!(after.contains("[mcp_servers.disko]"), "{}", after);
        assert!(after.contains("command = \"/Applications/Disko.app/Contents/MacOS/disko\""));
        // Re-install replaces, uninstall removes only ours.
        i.install(Target::Codex, Path::new("/x/disko")).unwrap();
        assert_eq!(std::fs::read_to_string(home.join(".codex/config.toml")).unwrap().matches("[mcp_servers.disko]").count(), 1);
        i.uninstall(Target::Codex).unwrap();
        let after = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(!after.contains("disko") && after.contains("[mcp_servers.other]"));

        // Fresh file: no empty [mcp_servers] header.
        let fresh = tmp("codex-fresh");
        let i = Installer::with_home(&fresh, None);
        i.install(Target::Codex, Path::new("/x/disko")).unwrap();
        let s = std::fs::read_to_string(fresh.join(".codex/config.toml")).unwrap();
        assert!(s.starts_with("[mcp_servers.disko]"), "{}", s);
        // Broken TOML is left alone.
        std::fs::write(fresh.join(".codex/config.toml"), "this = [broken").unwrap();
        assert!(i.install(Target::Codex, Path::new("/x/disko")).is_err());
        assert_eq!(std::fs::read_to_string(fresh.join(".codex/config.toml")).unwrap(), "this = [broken");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&fresh);
    }

    #[test]
    fn json_targets() {
        let home = tmp("json");
        std::fs::write(home.join(".claude.json"), r#"{"numStartups":3,"mcpServers":{"other":{"command":"o"}},"projects":{}}"#).unwrap();
        let i = Installer::with_home(&home, None);
        assert!(i.detected(Target::ClaudeCode));
        i.install(Target::ClaudeCode, Path::new("/x/disko")).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(home.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(v["numStartups"], 3);
        assert_eq!(v["mcpServers"]["other"]["command"], "o");
        assert_eq!(v["mcpServers"]["disko"]["args"][0], "--mcp");
        assert_eq!(v["mcpServers"]["disko"]["type"], "stdio");
        assert!(i.is_installed(Target::ClaudeCode));
        i.uninstall(Target::ClaudeCode).unwrap();
        assert!(!i.is_installed(Target::ClaudeCode));

        assert!(!i.detected(Target::Cursor) || Path::new("/Applications/Cursor.app").is_dir());
        i.install(Target::Cursor, Path::new("/x/disko")).unwrap();
        assert!(i.is_installed(Target::Cursor));
        let t = i.targets();
        assert_eq!(t.len(), 3);
        assert!(t.iter().find(|x| x.target == Target::Cursor).unwrap().installed);

        std::fs::write(home.join(".cursor/mcp.json"), "{broken").unwrap();
        assert!(i.install(Target::Cursor, Path::new("/x/disko")).is_err());
        assert!(i.install(Target::Cursor, Path::new("relative")).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_cli_is_preferred() {
        let home = tmp("cli");
        // A fake `claude` that records its arguments.
        let cli = home.join("fake-claude");
        std::fs::write(&cli, "#!/bin/sh\necho \"$@\" >> \"$HOME/calls.txt\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let i = Installer::with_home(&home, Some(cli));
        i.install(Target::ClaudeCode, Path::new("/x/Disko App/disko")).unwrap();
        let calls = std::fs::read_to_string(home.join("calls.txt")).unwrap();
        assert_eq!(calls, "mcp remove --scope user disko\nmcp add --scope user disko -- /x/Disko App/disko --mcp\n");
        assert!(!home.join(".claude.json").exists(), "the CLI owns ~/.claude.json");
        let _ = std::fs::remove_dir_all(&home);
    }
}
