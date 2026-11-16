//! The data-driven rule table the engine's single disk walk is matched against.
//!
//! * [`DIR_RULES`]: folders recognised by name plus proof next to them (a manifest sibling such as
//!   `package.json`, or a file inside such as `pyvenv.cfg`), limited to a scope (projects, app
//!   support, containers, the whole of ~/Library).
//! * [`FILE_RULES`]: big files (the engine reports files of at least 1 MiB): installers, unfinished
//!   downloads, core dumps, big personal files nobody opened in a year.
//! * [`PatternRule`]: rules added at runtime by the user, an agent or the community pack, as a glob
//!   relative to `~` or absolute. No code change needed.
//!
//! Locations the probes own (package stores, ~/Library/Caches/<app>, DerivedData …) are *claimed*:
//! the matcher tags them so the engine colours them and stops matching inside, but the finding
//! itself comes from the probe (with the tool's own command, version logic, etc.).

use disko_core::{Category, Safety, Source};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where a folder rule may match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Inside home, outside ~/Library, tool stores, app bundles, site-packages, node_modules.
    Projects,
    /// `~/Library/Application Support/<App>/…` (up to two levels down).
    AppSupport,
    /// `~/Library/Containers/<id>/Data/Library/Caches` and `~/Library/Group Containers/<id>/Library/Caches`.
    Containers,
    /// Anywhere inside ~/Library.
    Library,
}

/// How matched folders become findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    /// One finding per project (git root, else the folder's parent).
    Project,
    /// One finding per app (the folder right under Application Support, or the container).
    App,
    /// One finding per matched path.
    Single,
}

#[derive(Debug)]
pub struct DirRule {
    pub id: &'static str,
    /// Folder names. A trailing `*` matches a prefix ("target-*"), a leading one a suffix.
    pub names: &'static [&'static str],
    /// At least one of these must sit next to the folder (a leading `*` matches a suffix,
    /// "*.csproj"). Empty: no sibling needed.
    pub markers: &'static [&'static str],
    /// At least one of these must exist inside the folder. Empty: no check.
    pub inside: &'static [&'static str],
    /// What it is, short: "node_modules", "Cargo build", "Chromium cache".
    pub kind: &'static str,
    /// The tool that rebuilds it, for the `why` line: "npm install".
    pub rebuild: &'static str,
    pub icon: &'static str,
    pub category: Category,
    pub safety: Safety,
    pub regrows: bool,
    pub scope: Scope,
    pub group: Grouping,
    /// Process names that use it while they run in the project.
    pub tools: &'static [&'static str],
}

const NODE_TOOLS: &[&str] = &["node", "bun", "deno", "npm", "pnpm", "yarn", "next-server", "vite", "tsserver", "esbuild", "turbo"];
const RUST_TOOLS: &[&str] = &["cargo", "rustc", "rust-analyzer", "rust-analyzer-proc-macro-srv", "clippy-driver", "bacon", "cargo-watch"];
const SWIFT_TOOLS: &[&str] = &["swift-build", "swift-frontend", "sourcekit-lsp", "swift-test", "xcodebuild", "swift-package"];
const PY_TOOLS: &[&str] = &["python", "python3", "uv", "pytest", "pip", "jupyter", "ruff", "mypy", "poetry"];
const JVM_TOOLS: &[&str] = &["java", "gradle", "kotlin-daemon", "studio"];
const XCODE_TOOLS: &[&str] = &["xcodebuild", "XCBBuildService", "swift-frontend", "pod"];

macro_rules! dir {
    ($id:expr, $names:expr, $markers:expr, $inside:expr, $kind:expr, $rebuild:expr, $icon:expr, $cat:ident, $safety:ident, $scope:ident, $group:ident, $tools:expr) => {
        DirRule { id: $id, names: $names, markers: $markers, inside: $inside, kind: $kind, rebuild: $rebuild, icon: $icon, category: Category::$cat, safety: Safety::$safety, regrows: true, scope: Scope::$scope, group: Grouping::$group, tools: $tools }
    };
}

const GRADLE: &[&str] = &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"];
const CHROMIUM_PROFILE: &[&str] = &["Local State", "Preferences", "Cookies", "Local Storage", "Network", "Session Storage", "IndexedDB", "Code Cache", "GPUCache"];

pub static DIR_RULES: &[DirRule] = &[
    // --- project build output and dependencies ---
    dir!("build.node-modules", &["node_modules"], &["package.json"], &[], "node_modules", "npm, pnpm, yarn or bun install", "node", Build, Safe, Projects, Project, NODE_TOOLS),
    dir!("build.cargo-target", &["target"], &["Cargo.toml"], &["CACHEDIR.TAG", "debug", "release"], "Cargo build", "cargo build", "rust", Build, Safe, Projects, Project, RUST_TOOLS),
    dir!("build.cargo-target", &["target-*", "*-target", "target_*"], &[], &["CACHEDIR.TAG"], "Cargo build", "cargo build", "rust", Build, Safe, Projects, Project, RUST_TOOLS),
    dir!("build.swiftpm", &[".build"], &["Package.swift"], &[], "SwiftPM build", "swift build", "swift", Build, Safe, Projects, Project, SWIFT_TOOLS),
    dir!("build.derived-data", &["DerivedData"], &[], &["ModuleCache.noindex", "Build", "Index.noindex", "info.plist"], "Xcode build", "the next Xcode build", "xcode", Build, Safe, Projects, Project, XCODE_TOOLS),
    dir!("build.pods", &["Pods"], &["Podfile"], &[], "CocoaPods", "pod install", "xcode", Build, Safe, Projects, Project, XCODE_TOOLS),
    dir!("build.web", &[".next", ".nuxt", ".svelte-kit", ".turbo", ".parcel-cache", ".angular", ".vite", ".docusaurus", ".astro", ".output", ".nx", ".wrangler"], &["package.json", "turbo.json", "angular.json", "nx.json", "wrangler.toml"], &[], "web build cache", "the next dev server or build", "node", Build, Safe, Projects, Project, NODE_TOOLS),
    dir!("build.python-cache", &["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".hypothesis", ".pytype", ".pyre"], &[], &[], "Python caches", "the next run", "python", Build, Safe, Projects, Project, PY_TOOLS),
    dir!("build.python-env", &[".tox", ".nox"], &[], &[], "test environments", "tox or nox", "python", Build, Safe, Projects, Project, PY_TOOLS),
    dir!("build.venv", &[".venv", "venv", "env", ".env", "virtualenv"], &[], &["pyvenv.cfg"], "virtual environment", "uv sync, poetry install or pip install", "python", Build, Review, Projects, Project, PY_TOOLS),
    dir!("build.gradle", &[".gradle", "build", ".cxx", ".externalNativeBuild", ".kotlin"], GRADLE, &[], "Gradle build", "the next Gradle build", "java", Build, Safe, Projects, Project, JVM_TOOLS),
    dir!("build.dart", &[".dart_tool", "build"], &["pubspec.yaml"], &[], "Dart/Flutter build", "flutter pub get", "box", Build, Safe, Projects, Project, &["dart", "flutter"]),
    dir!("build.cmake", &["build", "cmake-build-debug", "cmake-build-release", "out", "_build"], &[], &["CMakeCache.txt"], "CMake build", "cmake", "box", Build, Safe, Projects, Project, &["cmake", "ninja", "make", "clang"]),
    dir!("build.zig", &["zig-cache", ".zig-cache", "zig-out"], &["build.zig"], &[], "Zig build", "zig build", "box", Build, Safe, Projects, Project, &["zig"]),
    dir!("build.haskell", &[".stack-work"], &["stack.yaml"], &[], "Stack build", "stack build", "box", Build, Safe, Projects, Project, &["stack", "ghc"]),
    dir!("build.haskell", &["dist-newstyle"], &["cabal.project", "*.cabal"], &[], "Cabal build", "cabal build", "box", Build, Safe, Projects, Project, &["cabal", "ghc"]),
    dir!("build.elm", &["elm-stuff"], &["elm.json"], &[], "Elm build", "elm make", "box", Build, Safe, Projects, Project, &["elm"]),
    dir!("build.elixir", &["_build", "deps"], &["mix.exs"], &[], "Mix build", "mix deps.get && mix compile", "box", Build, Safe, Projects, Project, &["beam.smp", "mix", "elixir"]),
    dir!("build.dotnet", &["bin", "obj"], &["*.csproj", "*.fsproj", "*.vbproj"], &[], ".NET build", "dotnet build", "box", Build, Safe, Projects, Project, &["dotnet"]),
    dir!("build.sam", &[".aws-sam", ".serverless"], &["template.yaml", "template.yml", "serverless.yml", "serverless.ts"], &[], "serverless build", "sam build / serverless package", "box", Build, Safe, Projects, Project, &[]),
    dir!("build.terraform", &[".terraform"], &["*.tf"], &[], "Terraform providers", "terraform init", "box", Build, Review, Projects, Project, &["terraform"]),
    dir!("build.bazel", &["bazel-out"], &["WORKSPACE", "MODULE.bazel", "WORKSPACE.bazel"], &[], "Bazel output", "bazel build", "box", Build, Safe, Projects, Project, &["bazel", "java"]),
    // --- caches Electron/Chromium apps keep beside their data ---
    dir!("cache.chromium", &["Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache", "GrShaderCache", "ShaderCache", "CacheStorage", "ScriptCache", "component_crx_cache", "Crashpad"], CHROMIUM_PROFILE, &[], "browser engine cache", "the app as you use it", "app", Cache, Safe, AppSupport, App, &[]),
    // --- sandboxed app caches ---
    dir!("cache.container", &["Caches"], &[], &[], "app cache", "the app as you use it", "app", Cache, Safe, Containers, App, &[]),
    // --- generic leaves anywhere in ~/Library (OnyX's best trick) ---
    dir!("cache.coreml", &["com.apple.e5rt.e5bundlecache"], &[], &[], "compiled Core ML models", "macOS on next use (first run is slower)", "box", Cache, Safe, Library, App, &[]),
    dir!("installers.sparkle", &["org.sparkle-project.Sparkle"], &[], &[], "update download", "the app's next update check", "app", Installers, Safe, Library, App, &[]),
    dir!("installers.shipit", &["*.ShipIt"], &[], &[], "update download", "the app's next update", "app", Installers, Safe, Library, App, &[]),
];

#[derive(Debug)]
pub struct FileRule {
    pub id: &'static str,
    /// Lowercase extensions (without the dot), or exact name prefixes ending in '*' ("core.*").
    pub ext: &'static [&'static str],
    pub category: Category,
    pub safety: Safety,
    pub icon: &'static str,
}

pub static FILE_RULES: &[FileRule] = &[
    FileRule { id: "installers.file", ext: &["dmg", "pkg", "mpkg", "xip", "ipsw", "iso", "msi", "exe"], category: Category::Installers, safety: Safety::Review, icon: "box" },
    FileRule { id: "installers.partial", ext: &["crdownload", "part", "partial", "download", "opdownload"], category: Category::Installers, safety: Safety::Review, icon: "box" },
    FileRule { id: "logs.core-dump", ext: &["core.*"], category: Category::Logs, safety: Safety::Safe, icon: "log" },
    FileRule { id: "forgotten.file", ext: &[], category: Category::Forgotten, safety: Safety::Review, icon: "file" },
];

/// Big personal files: this size, untouched this long.
pub const FORGOTTEN_MIN_BYTES: u64 = 500 * 1024 * 1024;
pub const FORGOTTEN_MIN_DAYS: i64 = 365;

/// A rule added at runtime. `pattern` is a glob relative to `~` ("~/Library/Caches/com.foo.*")
/// or absolute; `*` and `?` match within one path segment. It matches folders and files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternRule {
    pub id: String,
    pub pattern: String,
    pub category: Category,
    pub title: String,
    pub why: String,
    #[serde(default)]
    pub regrows: bool,
    pub safety: Safety,
    #[serde(default)]
    pub source: Source,
}

/// A compiled pattern: absolute segments.
#[derive(Debug, Clone)]
pub struct Glob {
    pub segs: Vec<String>,
    /// Leading literal part, for a cheap `starts_with` test and for expansion.
    pub prefix: PathBuf,
}

impl Glob {
    pub fn new(pattern: &str, home: &Path) -> Option<Glob> {
        let p = if let Some(rest) = pattern.strip_prefix("~/") {
            home.join(rest)
        } else if pattern == "~" {
            return None;
        } else {
            PathBuf::from(pattern)
        };
        if !p.is_absolute() || pattern.contains("..") || pattern.contains("**") {
            return None;
        }
        let segs: Vec<String> = p.components().skip(1).map(|c| c.as_os_str().to_string_lossy().to_string()).filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 {
            return None;
        }
        let mut prefix = PathBuf::from("/");
        for s in &segs {
            if s.contains(['*', '?', '[']) {
                break;
            }
            prefix.push(s);
        }
        Some(Glob { segs, prefix })
    }

    pub fn depth(&self) -> usize {
        self.segs.len()
    }

    pub fn matches(&self, path: &Path) -> bool {
        if !path.starts_with(&self.prefix) {
            return false;
        }
        let mut n = 0;
        for (c, seg) in path.components().skip(1).zip(self.segs.iter()) {
            if !wild(seg, &c.as_os_str().to_string_lossy()) {
                return false;
            }
            n += 1;
        }
        n == self.segs.len() && path.components().count() == self.segs.len() + 1
    }

    /// All existing paths the pattern matches (bounded), without a full walk.
    pub fn expand(&self, limit: usize) -> Vec<PathBuf> {
        let skip = self.prefix.components().count() - 1;
        let mut cur = vec![self.prefix.clone()];
        for seg in &self.segs[skip..] {
            let mut next = vec![];
            for dir in &cur {
                for c in crate::util::children(dir) {
                    if wild(seg, &crate::util::name_of(&c)) {
                        next.push(c);
                    }
                    if next.len() > limit {
                        break;
                    }
                }
            }
            cur = next;
        }
        if skip == self.segs.len() && !crate::util::exists(&self.prefix) {
            return vec![];
        }
        cur.retain(|p| crate::util::exists(p));
        cur.truncate(limit);
        cur
    }
}

/// `*` (any run) and `?` (one char) within a segment.
pub fn wild(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Name test for rule names with a leading or trailing `*`.
pub fn name_matches(rule: &str, name: &str) -> bool {
    if let Some(prefix) = rule.strip_suffix('*') {
        name.starts_with(prefix) && name.len() > prefix.len()
    } else if let Some(suffix) = rule.strip_prefix('*') {
        name.ends_with(suffix) && name.len() > suffix.len()
    } else {
        rule == name
    }
}

/// Home-relative folders whose insides are never project build output (tool stores and the like).
const NOT_PROJECTS: &[&str] = &[
    "Library", ".Trash", ".cargo", ".rustup", ".npm", ".nvm", ".bun", ".pyenv", ".gradle", ".m2", ".cocoapods", ".vscode", ".vscode-insiders", ".windsurf",
    ".cursor/extensions", ".local/share", ".local/pipx", ".local/lib", ".local/state", "go/pkg", "miniconda3", "anaconda3", "miniforge3", "mambaforge", ".conda", ".pnpm-store",
    ".yarn", ".deno", ".volta", ".fnm", ".android", ".pub-cache", ".dartServer", ".rbenv", ".gem", ".sdkman", ".asdf", ".mise", ".cache/uv", ".cache/pip", ".cache/huggingface",
    ".cache/pypoetry", ".cache/pre-commit", ".cache/bazel", ".cache/go-build", ".cache/ms-playwright", ".cache/puppeteer", ".swiftpm", ".orbstack", ".docker", "Applications",
    "Pictures", "Music", "Movies",
];

/// Inside home, outside tool stores, app bundles, site-packages, other dependencies.
pub fn in_projects(path: &Path, home: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(home) else { return false };
    if NOT_PROJECTS.iter().any(|x| rel.starts_with(x)) {
        return false;
    }
    // The folder itself may be called node_modules; its ancestors may not.
    let parent = rel.parent().unwrap_or(rel);
    !parent.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c == "node_modules" || c == "site-packages" || c == ".git" || c.ends_with(".app") || c == "vendor" || c == "Pods" || c == ".build" || c == "target"
    })
}

/// The scope test for a rule.
pub fn in_scope(scope: Scope, path: &Path, home: &Path) -> bool {
    match scope {
        Scope::Projects => in_projects(path, home),
        Scope::AppSupport => {
            let Ok(rel) = path.strip_prefix(home.join("Library/Application Support")) else { return false };
            let n = rel.components().count();
            (2..=4).contains(&n)
        }
        Scope::Containers => {
            let Ok(rel) = path.strip_prefix(home.join("Library")) else { return false };
            let c: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            (c.len() == 5 && c[0] == "Containers" && c[2] == "Data" && c[3] == "Library" && c[4] == "Caches")
                || (c.len() == 4 && c[0] == "Group Containers" && c[2] == "Library" && c[3] == "Caches")
        }
        Scope::Library => path.starts_with(home.join("Library")),
    }
}

/// Apple-owned sandboxed caches that regrow harmlessly (OnyX deletes the same ones).
pub const APPLE_CONTAINER_CACHES: &[&str] = &[
    "com.apple.mediaanalysisd",
    "com.apple.wallpaper.agent",
    "com.apple.AvatarUI.AvatarPickerMemojiPicker",
    "com.apple.helpviewer",
    "com.apple.tips",
    "com.apple.photoanalysisd",
    "com.apple.Maps",
    "com.apple.news",
    "com.apple.stocks",
    "com.apple.weather",
];

pub fn file_ext_matches(rule: &FileRule, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    rule.ext.iter().any(|e| if let Some(p) = e.strip_suffix('*') { lower.starts_with(p) } else { lower.ends_with(&format!(".{}", e)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wild("com.foo.*", "com.foo.bar"));
        assert!(wild("*.ShipIt", "com.x.ShipIt"));
        assert!(!wild("*.ShipIt", "com.x.ShipItX"));
        assert!(wild("a?c", "abc"));
        assert!(!wild("abc", "abcd"));
        assert!(name_matches("target-*", "target-agents"));
        assert!(!name_matches("target-*", "target-"));
    }

    #[test]
    fn globs_match_by_depth() {
        let home = Path::new("/Users/t");
        let g = Glob::new("~/Library/Caches/com.foo.*", home).unwrap();
        assert_eq!(g.prefix, PathBuf::from("/Users/t/Library/Caches"));
        assert!(g.matches(Path::new("/Users/t/Library/Caches/com.foo.app")));
        assert!(!g.matches(Path::new("/Users/t/Library/Caches/com.foo.app/x")));
        assert!(!g.matches(Path::new("/Users/t/Library/Caches")));
        assert!(Glob::new("relative/x", home).is_none());
        assert!(Glob::new("~/a/../b", home).is_none());
        assert!(Glob::new("/x", home).is_none());
    }

    #[test]
    fn project_scope() {
        let home = Path::new("/Users/t");
        assert!(in_projects(Path::new("/Users/t/dev/app/node_modules"), home));
        assert!(!in_projects(Path::new("/Users/t/Library/x/node_modules"), home));
        assert!(!in_projects(Path::new("/Users/t/.nvm/versions/node/v1/lib/node_modules"), home));
        assert!(!in_projects(Path::new("/Users/t/dev/app/node_modules/x/node_modules"), home));
        assert!(!in_projects(Path::new("/Users/t/dev/py/.venv/lib/python3.12/site-packages/x/__pycache__"), home));
        assert!(!in_projects(Path::new("/opt/homebrew/lib/node_modules"), home));
        assert!(in_projects(Path::new("/Users/t/.codex/worktrees/a/proj/target"), home));
    }
}
