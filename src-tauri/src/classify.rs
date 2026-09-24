//! Path classifier: base safety category plus orthogonal tag, action kind and recreate cost.
//!
//! Rules are ordered most-specific first. Safety overrides (things that look like cache but hold
//! user state) come before the generic prefixes, and manager-owned stores name the tool that
//! knows more about reachability than a timestamp does.
//!
//! Base categories answer "can this be proposed at all?":
//!   rebuildable  known cache or generated output, recreated on demand
//!   review       user data or manager-owned state that needs a human decision
//!   keep         app state, repositories, profiles; use the owning app to remove
//!   protected    system, credentials, cloud-managed, outside home; never touched
//!
//! Actions answer "how should it be removed?":
//!   quarantine       move the path into Disko's recoverable quarantine
//!   manager_command  the owning tool has safer garbage collection (see `tool`)
//!   owner_app        remove through the owning application's UI
//!   report_only      explain the space, do not offer removal

use std::{cell::RefCell, collections::HashMap, path::Path};

/// Bump when rule semantics change so cached reports regenerate.
pub const RULES_VERSION: u32 = 5;

#[derive(Debug, Clone)]
pub struct Classification {
    pub category: &'static str,
    pub tag: &'static str,
    pub action: &'static str,
    pub cost: &'static str,
    pub tool: Option<String>,
    pub reason: String,
}

impl Classification {
    fn new(category: &'static str, tag: &'static str, action: &'static str, cost: &'static str, reason: impl Into<String>) -> Self {
        Classification { category, tag, action, cost, tool: None, reason: reason.into() }
    }
}

const REBUILDABLE: &str = "rebuildable";
const REVIEW: &str = "review";
const KEEP: &str = "keep";
const PROTECTED: &str = "protected";

const QUARANTINE: &str = "quarantine";
const MANAGER: &str = "manager_command";
const OWNER_APP: &str = "owner_app";
const REPORT: &str = "report_only";

const INSTANT: &str = "instant";
const LOCAL: &str = "local-rebuild";
const NET_SMALL: &str = "network-small";
const NET_LARGE: &str = "network-large";
const UNKNOWN: &str = "unknown";
const IRREPLACEABLE: &str = "irreplaceable";

/// A static rule: home-relative path prefix (`*` matches any single segment), then the verdict.
struct Rule {
    prefix: &'static [&'static str],
    category: &'static str,
    tag: &'static str,
    action: &'static str,
    cost: &'static str,
    tool: Option<&'static str>,
    reason: &'static str,
}

macro_rules! rule {
    ($p:expr, $c:expr, $t:expr, $a:expr, $k:expr, $r:expr) => {
        Rule { prefix: $p, category: $c, tag: $t, action: $a, cost: $k, tool: None, reason: $r }
    };
    ($p:expr, $c:expr, $t:expr, $a:expr, $k:expr, $r:expr, tool = $tool:expr) => {
        Rule { prefix: $p, category: $c, tag: $t, action: $a, cost: $k, tool: Some($tool), reason: $r }
    };
}

/// Home-relative rules, first match wins. Keep exceptions above the generic prefix they override.
static HOME_RULES: &[Rule] = &[
    // --- credentials, privacy, cloud: never ---
    rule!(&[".ssh"], PROTECTED, "credential", REPORT, IRREPLACEABLE, "SSH keys and known hosts; never touched."),
    rule!(&[".gnupg"], PROTECTED, "credential", REPORT, IRREPLACEABLE, "GPG keyring; never touched."),
    rule!(&[".password-store"], PROTECTED, "credential", REPORT, IRREPLACEABLE, "Password store; never touched."),
    rule!(&[".aws"], PROTECTED, "credential", REPORT, IRREPLACEABLE, "Cloud credentials and profiles; never touched."),
    rule!(&["Library", "Keychains"], PROTECTED, "credential", REPORT, IRREPLACEABLE, "macOS keychains; never touched."),
    rule!(&["Library", "CloudStorage"], PROTECTED, "cloud-managed", REPORT, UNKNOWN, "Cloud-managed content; deleting here syncs the deletion to other devices."),
    rule!(&["Library", "Mobile Documents"], PROTECTED, "cloud-managed", REPORT, UNKNOWN, "iCloud Drive; deleting here syncs the deletion to other devices."),
    rule!(&["Library", "Mail"], KEEP, "tcc-gated", OWNER_APP, IRREPLACEABLE, "Mail messages and attachments; manage storage inside Mail."),
    rule!(&["Library", "Messages"], KEEP, "tcc-gated", OWNER_APP, IRREPLACEABLE, "Messages history and attachments; manage inside Messages."),
    rule!(&["Library", "Safari"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Safari history, bookmarks and tabs; clear from inside Safari."),
    rule!(&["Library", "Cookies"], KEEP, "tcc-gated", OWNER_APP, UNKNOWN, "Website logins and cookies; clear from the browser."),
    rule!(&["Library", "Accounts"], KEEP, "tcc-gated", OWNER_APP, IRREPLACEABLE, "Account credentials for Mail, Calendar and iCloud."),
    rule!(&["Library", "Photos"], KEEP, "managed-library", OWNER_APP, IRREPLACEABLE, "Photos library data; manage inside Photos."),
    rule!(&["Library", "Calendars"], KEEP, "tcc-gated", OWNER_APP, IRREPLACEABLE, "Calendar and reminders data."),
    rule!(&["Library", "Suggestions"], KEEP, "tcc-gated", REPORT, LOCAL, "Siri and Spotlight suggestion index; system managed."),
    rule!(&["Library", "Biome"], KEEP, "tcc-gated", REPORT, LOCAL, "On-device intelligence streams; system managed."),

    // --- device backups: whole-backup granularity only ---
    rule!(&["Library", "Application Support", "MobileSync", "Backup", "*"], REVIEW, "old-backup", OWNER_APP, IRREPLACEABLE, "An iPhone or iPad backup. Remove whole backups through Finder > Manage Backups if the device no longer needs it."),
    rule!(&["Library", "Application Support", "MobileSync"], KEEP, "old-backup", OWNER_APP, IRREPLACEABLE, "Device backup store; pick individual backups inside, never fragments."),

    // --- Docker and VM stores: the manager's object graph decides ---
    rule!(&["Library", "Containers", "com.docker.docker"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Docker Desktop's disk image holds images, build cache and volumes. Reclaim with Docker, never by moving the file.", tool = "docker system df -v && docker system prune"),
    rule!(&["Library", "Group Containers", "group.com.docker"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Docker Desktop shared state; manage through Docker.", tool = "docker system prune"),
    rule!(&[".docker"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Docker client config and contexts; use Docker to reclaim space.", tool = "docker system df -v"),
    rule!(&[".colima"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Colima VM disk; images and volumes live inside. Prune through Docker or delete the profile with Colima.", tool = "colima delete"),
    rule!(&[".lima"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Lima VM instances; manage with limactl.", tool = "limactl delete <instance>"),
    rule!(&[".minikube"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "minikube cluster state and images.", tool = "minikube delete"),
    rule!(&[".rd"], KEEP, "managed-container-store", MANAGER, UNKNOWN, "Rancher Desktop VM and images; reset from its settings."),
    rule!(&["Library", "Containers", "com.utmapp.UTM"], KEEP, "virtual-machine", OWNER_APP, IRREPLACEABLE, "UTM virtual machines; delete VMs inside UTM."),
    rule!(&["Parallels"], KEEP, "virtual-machine", OWNER_APP, IRREPLACEABLE, "Parallels virtual machines; a disk image is the whole VM."),
    rule!(&["Virtual Machines.localized"], KEEP, "virtual-machine", OWNER_APP, IRREPLACEABLE, "VMware Fusion virtual machines."),

    // --- Xcode and simulators ---
    rule!(&["Library", "Developer", "Xcode", "iOS DeviceSupport"], REVIEW, "device-support", QUARANTINE, NET_LARGE, "Debug symbols for physical iPhones and iPads. Review older OS versions in the Xcode page; old crash logs may still need these symbols."),
    rule!(&["Library", "Developer", "Xcode", "watchOS DeviceSupport"], REVIEW, "device-support", QUARANTINE, NET_LARGE, "Debug symbols for physical Apple Watches; keep versions you still debug."),
    rule!(&["Library", "Developer", "Xcode", "tvOS DeviceSupport"], REVIEW, "device-support", QUARANTINE, NET_LARGE, "Debug symbols for physical Apple TVs; keep versions you still debug."),
    rule!(&["Library", "Developer", "Xcode", "visionOS DeviceSupport"], REVIEW, "device-support", QUARANTINE, NET_LARGE, "Debug symbols for physical Vision Pro devices; keep versions you still debug."),
    rule!(&["Library", "Developer", "XcodeBuildMCP"], REVIEW, "agent-workspace", QUARANTINE, UNKNOWN, "Agent workspace copies. Stop the agent and check for changes that exist only in this copy before cleanup."),
    rule!(&["Library", "Developer", "Xcode", "DerivedData", "ModuleCache.noindex"], REBUILDABLE, "xcode-cache", QUARANTINE, LOCAL, "Clang module cache; rebuilt on the next compile."),
    rule!(&["Library", "Developer", "Xcode", "DerivedData"], REBUILDABLE, "build-output", QUARANTINE, LOCAL, "Xcode build output and indexes; close Xcode first, the next build recreates it."),
    rule!(&["Library", "Developer", "Xcode", "Archives"], REVIEW, "release-artifact", QUARANTINE, IRREPLACEABLE, "Release archives with dSYMs. Needed to symbolicate crashes and re-submit builds; cannot be recreated exactly."),
    rule!(&["Library", "Developer", "Xcode", "UserData"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Xcode snippets, breakpoints, themes and key bindings."),
    rule!(&["Library", "Developer", "Xcode", "DocumentationCache"], REBUILDABLE, "xcode-cache", QUARANTINE, NET_SMALL, "Documentation cache; Xcode re-downloads on demand."),
    rule!(&["Library", "Developer", "Xcode", "DocumentationIndex"], REBUILDABLE, "xcode-cache", QUARANTINE, LOCAL, "Documentation index; Xcode rebuilds it."),
    rule!(&["Library", "Developer", "Xcode", "Products"], REBUILDABLE, "build-output", QUARANTINE, LOCAL, "Built products; the next build recreates them."),
    rule!(&["Library", "Developer", "Xcode", "iOS Device Logs"], REVIEW, "diagnostic", QUARANTINE, UNKNOWN, "Device console logs collected by Xcode; keep anything from an open investigation."),
    rule!(&["Library", "Developer", "CoreSimulator", "Caches"], REBUILDABLE, "simulator-cache", QUARANTINE, LOCAL, "Simulator caches; quit Simulator and Xcode first."),
    rule!(&["Library", "Developer", "CoreSimulator", "Devices"], KEEP, "simulator-device", MANAGER, LOCAL, "Simulator devices can hold app data. Remove devices whose runtime is gone with simctl.", tool = "xcrun simctl delete unavailable"),
    rule!(&["Library", "Developer", "CoreSimulator", "Profiles", "Runtimes"], REVIEW, "unused-runtime", OWNER_APP, NET_LARGE, "Installed simulator runtimes. Remove old OS versions in Xcode > Settings > Components; each is a multi-GB download."),
    rule!(&["Library", "Developer", "CoreSimulator"], KEEP, "simulator-device", MANAGER, LOCAL, "Simulator state; manage through simctl or Xcode.", tool = "xcrun simctl list"),
    rule!(&["Library", "Developer", "XCTestDevices"], REVIEW, "test-runtime", QUARANTINE, LOCAL, "Simulator clones created for parallel testing; safe when no test run is active."),
    rule!(&["Library", "Developer", "Xcode", "DerivedData", "*"], REBUILDABLE, "build-output", QUARANTINE, LOCAL, "Build output for one project; rebuilt on the next build."),

    // --- JetBrains: cache root also holds Local History ---
    rule!(&["Library", "Caches", "JetBrains", "*"], REVIEW, "managed-cleanup", OWNER_APP, LOCAL, "JetBrains system directory: caches plus Local History for one IDE version. Use File > Invalidate Caches inside the IDE, or remove only if that IDE version is uninstalled."),
    rule!(&["Library", "Caches", "JetBrains"], KEEP, "managed-cleanup", OWNER_APP, LOCAL, "JetBrains cache roots per IDE version; contains Local History."),
    rule!(&["Library", "Logs", "JetBrains", "*"], REVIEW, "stale-log", QUARANTINE, INSTANT, "Logs for one JetBrains IDE version; safe once that version is no longer installed."),
    rule!(&["Library", "Application Support", "JetBrains"], KEEP, "app-state", OWNER_APP, IRREPLACEABLE, "JetBrains settings, plugins and licenses."),

    // --- browsers: cache and profile are different things ---
    rule!(&["Library", "Caches", "Google", "Chrome"], REBUILDABLE, "browser-cache", QUARANTINE, NET_SMALL, "Chrome disk cache; quit Chrome first. Bookmarks and history live elsewhere."),
    rule!(&["Library", "Caches", "Firefox"], REBUILDABLE, "browser-cache", QUARANTINE, NET_SMALL, "Firefox disk cache; quit Firefox first."),
    rule!(&["Library", "Caches", "BraveSoftware"], REBUILDABLE, "browser-cache", QUARANTINE, NET_SMALL, "Brave disk cache; quit Brave first."),
    rule!(&["Library", "Caches", "Microsoft Edge"], REBUILDABLE, "browser-cache", QUARANTINE, NET_SMALL, "Edge disk cache; quit Edge first."),
    rule!(&["Library", "Caches", "company.thebrowser.Browser"], REBUILDABLE, "browser-cache", QUARANTINE, NET_SMALL, "Arc disk cache; quit Arc first."),
    rule!(&["Library", "Application Support", "Google", "Chrome"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Chrome profiles: history, bookmarks, cookies, passwords. Remove profiles inside Chrome."),
    rule!(&["Library", "Application Support", "Firefox"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Firefox profiles persist across uninstall by design; they hold bookmarks and passwords."),
    rule!(&["Library", "Application Support", "BraveSoftware"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Brave profiles and wallet data."),
    rule!(&["Library", "Application Support", "Microsoft Edge"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Edge profiles and saved data."),
    rule!(&["Library", "Application Support", "Arc"], KEEP, "browser-profile", OWNER_APP, IRREPLACEABLE, "Arc spaces, tabs and profile data."),

    // --- manager-owned caches under Library/Caches ---
    rule!(&["Library", "Caches", "Homebrew"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Homebrew download cache. Homebrew knows which bottles are stale.", tool = "brew cleanup --prune=all"),
    rule!(&["Library", "Caches", "pip"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pip wheel and HTTP cache.", tool = "python3 -m pip cache purge"),
    rule!(&["Library", "Caches", "pnpm"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pnpm metadata cache.", tool = "pnpm store prune"),
    rule!(&["Library", "pnpm"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pnpm content-addressable store. Prune removes only packages no project references.", tool = "pnpm store prune"),
    rule!(&["Library", "Caches", "Yarn"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Yarn package cache.", tool = "yarn cache clean"),
    rule!(&["Library", "Caches", "go-build"], REBUILDABLE, "build-cache", MANAGER, LOCAL, "Go build cache.", tool = "go clean -cache"),
    rule!(&["Library", "Caches", "ms-playwright"], REBUILDABLE, "managed-cleanup", MANAGER, NET_LARGE, "Playwright browser builds; re-downloaded on next install, several hundred MB each.", tool = "npx playwright uninstall --all"),
    rule!(&["Library", "Caches", "Cypress"], REBUILDABLE, "managed-cleanup", MANAGER, NET_LARGE, "Cypress binary versions.", tool = "npx cypress cache prune"),
    rule!(&["Library", "Caches", "CocoaPods"], REBUILDABLE, "dependency-cache", MANAGER, NET_SMALL, "CocoaPods spec and pod cache.", tool = "pod cache clean --all"),
    rule!(&["Library", "Caches", "org.carthage.CarthageKit"], REBUILDABLE, "dependency-cache", QUARANTINE, NET_SMALL, "Carthage dependency checkouts and builds."),
    rule!(&["Library", "Caches", "org.swift.swiftpm"], REBUILDABLE, "dependency-cache", QUARANTINE, NET_SMALL, "Swift Package Manager repository cache; re-cloned on resolve."),
    rule!(&["Library", "Caches", "com.apple.dt.Xcode"], REBUILDABLE, "xcode-cache", QUARANTINE, LOCAL, "Xcode's own cache; quit Xcode first."),
    rule!(&["Library", "Caches", "com.spotify.client"], REBUILDABLE, "app-cache", QUARANTINE, NET_LARGE, "Spotify cache including offline downloads; songs re-download."),
    rule!(&["Library", "Caches", "CloudKit"], KEEP, "app-cache", REPORT, LOCAL, "iCloud sync caches; system managed."),
    rule!(&["Library", "Caches", "com.apple.Safari"], REBUILDABLE, "browser-cache", OWNER_APP, NET_SMALL, "Safari cache; clear from Safari > Settings > Privacy."),
    rule!(&["Library", "Caches"], REBUILDABLE, "app-cache", QUARANTINE, LOCAL, "App cache; quit the owning app first. Some apps keep offline data here."),

    // --- Library state that is actually regenerable ---
    rule!(&["Library", "Saved Application State"], REBUILDABLE, "app-state", QUARANTINE, INSTANT, "Window positions and restore state; recreated on next launch. Unsaved-document recovery data can be here."),
    rule!(&["Library", "Logs", "DiagnosticReports"], REVIEW, "diagnostic", QUARANTINE, INSTANT, "Crash and spin reports. Keep recent ones if you are troubleshooting."),
    rule!(&["Library", "Logs"], REVIEW, "diagnostic", QUARANTINE, INSTANT, "Application logs; useful while debugging, otherwise disposable."),
    rule!(&["Library", "Application Support", "CrashReporter"], REVIEW, "diagnostic", QUARANTINE, INSTANT, "Crash reporter state."),
    rule!(&["Library", "HTTPStorages"], REBUILDABLE, "app-cache", QUARANTINE, INSTANT, "Per-app HTTP caches and cookies; apps recreate them."),
    rule!(&["Library", "Containers", "*", "Data", "Library", "Caches"], REBUILDABLE, "app-cache", QUARANTINE, LOCAL, "Cache of a sandboxed app; quit the app first."),
    rule!(&["Library", "Group Containers", "*", "Library", "Caches"], REBUILDABLE, "app-cache", QUARANTINE, LOCAL, "Shared cache of a sandboxed app group; quit the apps first."),
    rule!(&["Library", "Containers", "*"], KEEP, "app-state", OWNER_APP, IRREPLACEABLE, "Sandboxed app container: documents, settings and databases of one app."),
    rule!(&["Library", "Group Containers", "*"], KEEP, "shared-app-state", OWNER_APP, IRREPLACEABLE, "Shared state between several apps of one vendor; never assume one app owns it."),

    // --- Android ---
    rule!(&["Library", "Android", "sdk", "system-images"], REVIEW, "unused-runtime", MANAGER, NET_LARGE, "Android emulator system images; remove unneeded API levels with SDK Manager.", tool = "sdkmanager --uninstall <package>"),
    rule!(&["Library", "Android", "sdk", "platforms"], REVIEW, "unused-runtime", MANAGER, NET_LARGE, "Android platform SDKs per API level.", tool = "sdkmanager --uninstall <package>"),
    rule!(&["Library", "Android", "sdk", "build-tools"], REVIEW, "unused-runtime", MANAGER, NET_LARGE, "Android build tools per version.", tool = "sdkmanager --uninstall <package>"),
    rule!(&["Library", "Android", "sdk", "ndk"], REVIEW, "unused-runtime", MANAGER, NET_LARGE, "Android NDK versions.", tool = "sdkmanager --uninstall <package>"),
    rule!(&["Library", "Android", "sdk"], KEEP, "managed-cleanup", MANAGER, NET_LARGE, "Android SDK; manage components with SDK Manager.", tool = "sdkmanager --list_installed"),
    rule!(&[".android", "avd"], REVIEW, "virtual-device", MANAGER, LOCAL, "Android virtual devices with their app data; delete with AVD Manager.", tool = "avdmanager delete avd -n <name>"),
    rule!(&[".android", "cache"], REBUILDABLE, "app-cache", QUARANTINE, LOCAL, "Android tooling cache."),
    rule!(&[".gradle", "caches"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Gradle dependency and build caches; Gradle expires them on its own schedule.", tool = "gradle --stop && rm -rf ~/.gradle/caches"),
    rule!(&[".gradle", "wrapper", "dists"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_LARGE, "Downloaded Gradle distributions, one per version used."),
    rule!(&[".gradle", "daemon"], REVIEW, "stale-log", QUARANTINE, INSTANT, "Gradle daemon logs."),
    rule!(&[".m2", "repository"], REVIEW, "dependency-repository", QUARANTINE, NET_SMALL, "Maven local repository. Mostly re-downloadable, but locally installed artifacts exist nowhere else."),

    // --- Rust, Go ---
    rule!(&[".cargo", "registry"], REBUILDABLE, "dependency-cache", QUARANTINE, NET_SMALL, "Cargo crate downloads, sources and index; re-fetched on the next build."),
    rule!(&[".cargo", "git"], REBUILDABLE, "dependency-cache", QUARANTINE, NET_SMALL, "Cargo git dependency checkouts; re-cloned on the next build."),
    rule!(&[".cargo", "bin"], KEEP, "app-state", MANAGER, NET_SMALL, "Installed Cargo binaries.", tool = "cargo uninstall <crate>"),
    rule!(&[".cargo"], KEEP, "app-state", REPORT, UNKNOWN, "Cargo home: config and credentials."),
    rule!(&[".rustup", "toolchains", "*"], REVIEW, "unused-runtime", MANAGER, NET_LARGE, "A Rust toolchain. Keep the default and any pinned by rust-toolchain files; uninstall the rest with rustup.", tool = "rustup toolchain uninstall <toolchain>"),
    rule!(&[".rustup", "downloads"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "rustup download cache."),
    rule!(&[".rustup", "tmp"], REBUILDABLE, "managed-cleanup", QUARANTINE, INSTANT, "rustup temporary files."),
    rule!(&[".rustup"], KEEP, "managed-cleanup", MANAGER, NET_LARGE, "rustup home; manage toolchains with rustup.", tool = "rustup show"),
    rule!(&["go", "pkg", "mod"], REBUILDABLE, "dependency-cache", MANAGER, NET_SMALL, "Go module cache.", tool = "go clean -modcache"),
    rule!(&["go", "pkg"], REBUILDABLE, "dependency-cache", MANAGER, NET_SMALL, "Go package cache.", tool = "go clean -modcache"),

    // --- Node runtimes and package managers ---
    rule!(&[".npm", "_cacache"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "npm package download cache.", tool = "npm cache clean --force"),
    rule!(&[".npm", "_npx"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "Packages installed temporarily by npx."),
    rule!(&[".npm", "_logs"], REVIEW, "stale-log", QUARANTINE, INSTANT, "npm diagnostic logs."),
    rule!(&[".pnpm-store"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pnpm content-addressable store.", tool = "pnpm store prune"),
    rule!(&[".yarn", "cache"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Yarn package cache.", tool = "yarn cache clean"),
    rule!(&[".yarn", "berry", "cache"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Yarn Berry global cache.", tool = "yarn cache clean --mirror"),
    rule!(&[".bun", "install", "cache"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Bun global package cache.", tool = "bun pm cache rm"),
    rule!(&[".nvm", "versions", "node", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A Node.js version with its global packages. Keep the default and any referenced by .nvmrc files.", tool = "nvm uninstall <version>"),
    rule!(&[".nvm", ".cache"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "nvm download cache."),
    rule!(&[".fnm", "node-versions", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A Node.js version managed by fnm.", tool = "fnm uninstall <version>"),
    rule!(&[".volta", "tools", "image", "node", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A Node.js version managed by Volta.", tool = "volta uninstall node@<version>"),

    // --- Python ---
    rule!(&[".cache", "uv"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "uv cache; uv tracks which entries are still referenced.", tool = "uv cache prune"),
    rule!(&[".local", "share", "uv", "python", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A Python version installed by uv.", tool = "uv python uninstall <version>"),
    rule!(&[".local", "share", "uv", "tools", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A tool environment installed by uv.", tool = "uv tool uninstall <tool>"),
    rule!(&[".cache", "pip"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pip cache.", tool = "python3 -m pip cache purge"),
    rule!(&[".cache", "pypoetry"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Poetry artifact cache.", tool = "poetry cache clear --all ."),
    rule!(&[".pyenv", "versions", "*"], REVIEW, "unused-runtime", MANAGER, NET_SMALL, "A Python version built by pyenv, with its site-packages.", tool = "pyenv uninstall <version>"),
    rule!(&[".pyenv", "cache"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "pyenv source tarballs."),
    rule!(&["miniconda3", "pkgs"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Conda package cache. Normal clean is safe; --force-pkgs-dirs is not.", tool = "conda clean --all"),
    rule!(&["anaconda3", "pkgs"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Conda package cache.", tool = "conda clean --all"),
    rule!(&["miniforge3", "pkgs"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Conda package cache.", tool = "conda clean --all"),
    rule!(&["mambaforge", "pkgs"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Conda package cache.", tool = "conda clean --all"),
    rule!(&[".conda", "pkgs"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Conda package cache.", tool = "conda clean --all"),
    rule!(&["miniconda3", "envs", "*"], REVIEW, "generated-environment", MANAGER, NET_SMALL, "A Conda environment; recreate from its environment file.", tool = "conda env remove -n <name>"),
    rule!(&["anaconda3", "envs", "*"], REVIEW, "generated-environment", MANAGER, NET_SMALL, "A Conda environment.", tool = "conda env remove -n <name>"),
    rule!(&["miniforge3", "envs", "*"], REVIEW, "generated-environment", MANAGER, NET_SMALL, "A Conda environment.", tool = "conda env remove -n <name>"),

    // --- models ---
    rule!(&[".cache", "huggingface", "hub"], REBUILDABLE, "model-cache", MANAGER, NET_LARGE, "Hugging Face model and dataset revisions. Delete per revision; re-downloads can be tens of GB.", tool = "hf cache delete"),
    rule!(&[".cache", "huggingface", "xet"], REBUILDABLE, "model-cache", QUARANTINE, NET_LARGE, "Hugging Face Xet chunk cache."),
    rule!(&[".cache", "huggingface"], REBUILDABLE, "model-cache", MANAGER, NET_LARGE, "Hugging Face caches.", tool = "hf cache scan"),
    rule!(&[".cache", "torch"], REBUILDABLE, "model-cache", QUARANTINE, NET_LARGE, "PyTorch hub checkpoints; re-downloaded on use."),
    rule!(&[".cache", "whisper"], REBUILDABLE, "model-cache", QUARANTINE, NET_LARGE, "Whisper model weights; re-downloaded on use."),
    rule!(&[".ollama", "models"], REVIEW, "downloaded-model", MANAGER, NET_LARGE, "Ollama model weights, often tens of GB. Remove individual models with Ollama.", tool = "ollama list && ollama rm <model>"),
    rule!(&[".ollama"], KEEP, "app-state", REPORT, UNKNOWN, "Ollama configuration and keys."),

    // --- generic tool caches ---
    rule!(&[".cache", "ms-playwright"], REBUILDABLE, "managed-cleanup", MANAGER, NET_LARGE, "Playwright browser builds.", tool = "npx playwright uninstall --all"),
    rule!(&[".cache", "puppeteer"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_LARGE, "Puppeteer Chromium builds; re-downloaded on install."),
    rule!(&[".cache", "Cypress"], REBUILDABLE, "managed-cleanup", MANAGER, NET_LARGE, "Cypress binary versions.", tool = "npx cypress cache prune"),
    rule!(&[".cache", "pre-commit"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "pre-commit hook environments.", tool = "pre-commit gc"),
    rule!(&[".cache", "bazel"], REBUILDABLE, "build-cache", QUARANTINE, LOCAL, "Bazel output base; rebuilt on the next build."),
    rule!(&[".cache", "go-build"], REBUILDABLE, "build-cache", MANAGER, LOCAL, "Go build cache.", tool = "go clean -cache"),
    rule!(&[".cache", "node-gyp"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "Node headers for native module builds."),
    rule!(&[".cache", "typescript"], REBUILDABLE, "managed-cleanup", QUARANTINE, NET_SMALL, "Automatic type acquisition cache."),
    rule!(&[".cache", "yarn"], REBUILDABLE, "managed-cleanup", MANAGER, NET_SMALL, "Yarn cache.", tool = "yarn cache clean"),
    rule!(&[".cache"], REBUILDABLE, "app-cache", QUARANTINE, LOCAL, "Tool cache; stop the owning tool first and check for offline assets."),

    // --- agent and tool state ---
    rule!(&[".codex"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Codex sessions and configuration."),
    rule!(&[".claude"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Claude Code sessions, memory and configuration."),
    rule!(&[".agents"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Agent configuration."),
    rule!(&[".config"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Tool configuration."),
    rule!(&[".local", "share"], KEEP, "app-state", REPORT, IRREPLACEABLE, "Persistent tool data."),
    rule!(&[".Trash"], PROTECTED, "existing-trash", REPORT, UNKNOWN, "Already in the Trash; empty the Trash to free it."),

];

/// Top-level user folders, checked after generated project output so a node_modules inside
/// Documents is still build output.
static USER_RULES: &[Rule] = &[
    rule!(&["Downloads"], REVIEW, "downloads", QUARANTINE, UNKNOWN, "Downloads are user files; verify installers, archives and originals individually."),
    rule!(&["Desktop"], REVIEW, "user-files", QUARANTINE, IRREPLACEABLE, "User files on the Desktop."),
    rule!(&["Documents"], REVIEW, "user-files", QUARANTINE, IRREPLACEABLE, "User documents."),
    rule!(&["Pictures"], REVIEW, "user-files", QUARANTINE, IRREPLACEABLE, "User pictures."),
    rule!(&["Movies"], REVIEW, "user-files", QUARANTINE, IRREPLACEABLE, "User videos."),
    rule!(&["Music"], REVIEW, "user-files", OWNER_APP, IRREPLACEABLE, "Music library. Downloaded Apple Music tracks can be removed inside Music and streamed again."),
];

/// Absolute-path rules for things outside the home directory (audit only under the current model).
static SYSTEM_RULES: &[(&[&str], &str, &str, &str, Option<&str>, &str)] = &[
    (&["private", "var", "vm"], "system-managed", REPORT, UNKNOWN, None, "Swap and sleep image; macOS manages this."),
    (&["private", "var", "folders"], "system-managed", REPORT, LOCAL, None, "Per-user system caches and temp files. macOS clears these; a Safe Mode boot flushes them."),
    (&["private", "var", "log"], "system-log", REPORT, INSTANT, None, "System logs; retention is managed by the OS."),
    (&["private", "var", "db"], "system-managed", REPORT, UNKNOWN, None, "System databases."),
    (&["System", "Volumes", "VM"], "system-managed", REPORT, UNKNOWN, None, "APFS VM volume for swap."),
    (&["System"], "system-managed", REPORT, UNKNOWN, None, "macOS system volume; sealed and SIP-protected."),
    (&["opt", "homebrew"], "managed-cleanup", MANAGER, NET_SMALL, Some("brew cleanup --dry-run"), "Homebrew prefix. Stale kegs and downloads are reclaimed by Homebrew itself."),
    (&["usr", "local", "Homebrew"], "managed-cleanup", MANAGER, NET_SMALL, Some("brew cleanup --dry-run"), "Homebrew prefix (Intel). Use brew cleanup."),
    (&["usr", "local", "Cellar"], "managed-cleanup", MANAGER, NET_SMALL, Some("brew cleanup --dry-run"), "Homebrew kegs (Intel). Use brew cleanup."),
    (&["Library", "Application Support", "Logic"], "redownloadable-apple-content", OWNER_APP, NET_LARGE, None, "Logic Pro sound library; remove packs in Logic > Sound Library."),
    (&["Library", "Application Support", "GarageBand"], "redownloadable-apple-content", OWNER_APP, NET_LARGE, None, "GarageBand sound library; remove packs in GarageBand > Sound Library."),
    (&["Library", "Audio", "Apple Loops"], "redownloadable-apple-content", OWNER_APP, NET_LARGE, None, "Apple Loops; managed by Logic and GarageBand."),
    (&["Library", "Developer", "CoreSimulator"], "unused-runtime", OWNER_APP, NET_LARGE, None, "System-wide simulator runtimes; manage in Xcode > Settings > Components."),
    (&["Library", "LaunchAgents"], "orphaned-service", REPORT, UNKNOWN, None, "System-wide launch agents; audit only."),
    (&["Library", "LaunchDaemons"], "orphaned-service", REPORT, UNKNOWN, None, "System-wide launch daemons; audit only."),
    (&["Library", "PrivilegedHelperTools"], "privileged-helper", REPORT, UNKNOWN, None, "Privileged helpers installed by apps; audit only."),
    (&["Applications"], "application", OWNER_APP, NET_LARGE, None, "Installed applications; uninstall through the app or its vendor uninstaller."),
    (&["Users"], "old-user-data", REPORT, IRREPLACEABLE, None, "Another user's home; may belong to a deleted account."),
];

/// Directory names anywhere in the tree that are generated from project sources. `marker` is a
/// sibling file that proves the project can regenerate it; `locked` is the lockfile that makes the
/// regeneration exact.
struct Generated {
    name: &'static str,
    marker: &'static [&'static str],
    locked: &'static [&'static str],
    tag: &'static str,
    cost: &'static str,
    reason: &'static str,
    /// Category when the marker is present; without a marker the entry falls back to `review`.
    category: &'static str,
}

static GENERATED: &[Generated] = &[
    Generated { name: "node_modules", marker: &["package.json"], locked: &["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb"], tag: "generated-project-output", cost: NET_SMALL, reason: "Installed npm dependencies", category: REBUILDABLE },
    Generated { name: "target", marker: &["Cargo.toml"], locked: &["Cargo.lock"], tag: "build-output", cost: LOCAL, reason: "Cargo build output", category: REBUILDABLE },
    Generated { name: ".build", marker: &["Package.swift"], locked: &["Package.resolved"], tag: "build-output", cost: LOCAL, reason: "Swift Package Manager build output", category: REBUILDABLE },
    Generated { name: ".next", marker: &["package.json"], locked: &[], tag: "build-output", cost: LOCAL, reason: "Next.js build output", category: REBUILDABLE },
    Generated { name: ".nuxt", marker: &["package.json"], locked: &[], tag: "build-output", cost: LOCAL, reason: "Nuxt build output", category: REBUILDABLE },
    Generated { name: ".turbo", marker: &["package.json"], locked: &[], tag: "build-cache", cost: LOCAL, reason: "Turborepo cache", category: REBUILDABLE },
    Generated { name: ".parcel-cache", marker: &["package.json"], locked: &[], tag: "build-cache", cost: LOCAL, reason: "Parcel cache", category: REBUILDABLE },
    Generated { name: ".svelte-kit", marker: &["package.json"], locked: &[], tag: "build-output", cost: LOCAL, reason: "SvelteKit build output", category: REBUILDABLE },
    Generated { name: ".angular", marker: &["angular.json"], locked: &[], tag: "build-cache", cost: LOCAL, reason: "Angular CLI cache", category: REBUILDABLE },
    Generated { name: "Pods", marker: &["Podfile"], locked: &["Podfile.lock"], tag: "generated-project-output", cost: NET_SMALL, reason: "CocoaPods checkouts", category: REBUILDABLE },
    Generated { name: ".gradle", marker: &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"], locked: &[], tag: "project-cache", cost: LOCAL, reason: "Gradle project cache", category: REBUILDABLE },
    Generated { name: "DerivedData", marker: &[], locked: &[], tag: "build-output", cost: LOCAL, reason: "Xcode build output", category: REBUILDABLE },
    Generated { name: ".dart_tool", marker: &["pubspec.yaml"], locked: &["pubspec.lock"], tag: "build-cache", cost: LOCAL, reason: "Dart tool cache", category: REBUILDABLE },
    Generated { name: "__pycache__", marker: &[], locked: &[], tag: "build-cache", cost: INSTANT, reason: "Python bytecode cache", category: REBUILDABLE },
    Generated { name: ".pytest_cache", marker: &[], locked: &[], tag: "build-cache", cost: INSTANT, reason: "pytest cache", category: REBUILDABLE },
    Generated { name: ".mypy_cache", marker: &[], locked: &[], tag: "build-cache", cost: LOCAL, reason: "mypy cache", category: REBUILDABLE },
    Generated { name: ".ruff_cache", marker: &[], locked: &[], tag: "build-cache", cost: INSTANT, reason: "ruff cache", category: REBUILDABLE },
    Generated { name: ".tox", marker: &["tox.ini", "pyproject.toml"], locked: &[], tag: "generated-environment", cost: NET_SMALL, reason: "tox environments", category: REBUILDABLE },
    Generated { name: ".nox", marker: &["noxfile.py"], locked: &[], tag: "generated-environment", cost: NET_SMALL, reason: "nox environments", category: REBUILDABLE },
    Generated { name: ".eggs", marker: &[], locked: &[], tag: "build-cache", cost: NET_SMALL, reason: "setuptools egg cache", category: REBUILDABLE },
    Generated { name: ".ipynb_checkpoints", marker: &[], locked: &[], tag: "build-cache", cost: INSTANT, reason: "Jupyter checkpoints", category: REBUILDABLE },
    Generated { name: "zig-cache", marker: &["build.zig"], locked: &[], tag: "build-cache", cost: LOCAL, reason: "Zig build cache", category: REBUILDABLE },
    Generated { name: ".zig-cache", marker: &["build.zig"], locked: &[], tag: "build-cache", cost: LOCAL, reason: "Zig build cache", category: REBUILDABLE },
    Generated { name: "zig-out", marker: &["build.zig"], locked: &[], tag: "build-output", cost: LOCAL, reason: "Zig build output", category: REBUILDABLE },
    Generated { name: ".venv", marker: &["pyproject.toml", "requirements.txt", "Pipfile", "setup.py"], locked: &["uv.lock", "poetry.lock", "Pipfile.lock", "requirements.lock"], tag: "generated-environment", cost: NET_SMALL, reason: "Python virtual environment", category: REVIEW },
    Generated { name: "venv", marker: &["pyproject.toml", "requirements.txt", "Pipfile", "setup.py"], locked: &["uv.lock", "poetry.lock", "Pipfile.lock"], tag: "generated-environment", cost: NET_SMALL, reason: "Python virtual environment", category: REVIEW },
    Generated { name: ".terraform", marker: &[], locked: &[".terraform.lock.hcl"], tag: "dependency-cache", cost: NET_SMALL, reason: "Terraform providers and modules; local state backends can also live here", category: REVIEW },
];

const INSTALLER_EXT: &[&str] = &["dmg", "pkg", "mpkg", "xip", "iso", "zip", "tar.gz", "tgz", "app.zip"];

thread_local! {
    static MARKER_CACHE: RefCell<HashMap<String, (bool, bool)>> = RefCell::new(HashMap::new());
}

/// (marker present, lockfile present) for a project directory, cached per thread because the
/// scanner asks about every entry under `node_modules`.
fn project_markers(project_dir: &str, g: &Generated) -> (bool, bool) {
    let key = format!("{}\u{0}{}", project_dir, g.name);
    MARKER_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(&key) {
            return *v;
        }
        let dir = Path::new(project_dir);
        let marker = g.marker.is_empty() || g.marker.iter().any(|m| dir.join(m).exists());
        let locked = g.locked.iter().any(|m| dir.join(m).exists());
        let mut cache = c.borrow_mut();
        if cache.len() > 50_000 {
            cache.clear();
        }
        cache.insert(key, (marker, locked));
        (marker, locked)
    })
}

fn matches_prefix(parts: &[&str], prefix: &[&str]) -> bool {
    parts.len() >= prefix.len() && prefix.iter().zip(parts).all(|(p, s)| *p == "*" || p == s)
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn classify(path: &str, home: &str) -> Classification {
    let home = home.trim_end_matches('/');
    let inside = path == home || path.starts_with(&format!("{}/", home));
    if !inside {
        return classify_outside(path);
    }
    let rel = path.strip_prefix(home).unwrap_or("").trim_start_matches('/');
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Classification::new(PROTECTED, "home", REPORT, IRREPLACEABLE, "Home folder; preserve.");
    }
    // Credentials anywhere in the tree.
    if parts.iter().any(|p| *p == ".ssh" || *p == ".gnupg") {
        return Classification::new(PROTECTED, "credential", REPORT, IRREPLACEABLE, "Credentials; never touched.");
    }
    if parts.iter().any(|p| *p == ".git" || *p == ".svn" || *p == ".hg") {
        return Classification::new(KEEP, "repository", REPORT, IRREPLACEABLE, "Version control history; use repository-aware maintenance such as git gc.");
    }
    if let Some(p) = parts.iter().find(|p| p.ends_with(".photoslibrary") || p.ends_with(".photolibrary") || p.ends_with(".musiclibrary") || p.ends_with(".fcpbundle") || p.ends_with(".imovielibrary") || p.ends_with(".tvlibrary")) {
        let reason = if p.ends_with(".fcpbundle") || p.ends_with(".imovielibrary") {
            "Video library. Render, optimized and proxy media can be regenerated: use File > Delete Generated Library Files inside the app."
        } else {
            "Managed media library; use its owning application."
        };
        return Classification::new(KEEP, "managed-library", OWNER_APP, IRREPLACEABLE, reason);
    }

    for r in HOME_RULES {
        if matches_prefix(&parts, r.prefix) {
            let mut c = Classification::new(r.category, r.tag, r.action, r.cost, r.reason);
            if let Some(t) = r.tool {
                c.tool = Some(t.to_string());
            }
            // Per-project DerivedData children carry the project name in front of a hash.
            if r.prefix.len() == 5 && r.prefix[3] == "DerivedData" && parts.len() >= 5 {
                let name = parts[4].rsplit_once('-').map(|(n, _)| n).unwrap_or(parts[4]);
                c.reason = format!("Build output for {}; rebuilt on the next build.", name);
            }
            return c;
        }
    }

    // Generated project output anywhere in the tree (first occurrence decides).
    for (idx, seg) in parts.iter().enumerate() {
        if let Some(g) = GENERATED.iter().find(|g| g.name == *seg) {
            let project_dir = format!("{}/{}", home, parts[..idx].join("/"));
            let (marker, locked) = project_markers(&project_dir, g);
            let project = if idx == 0 { "home".to_string() } else { parts[idx - 1].to_string() };
            if !marker {
                return Classification::new(REVIEW, g.tag, QUARANTINE, g.cost, format!("{} without a recognizable project next to it; verify before removing.", g.reason));
            }
            let reason = if locked {
                format!("{} for {}; a lockfile makes reinstalling exact.", g.reason, project)
            } else if g.locked.is_empty() {
                format!("{} for {}; regenerated by the next build.", g.reason, project)
            } else {
                format!("{} for {}; no lockfile found, so a reinstall may resolve different versions.", g.reason, project)
            };
            let category = if g.category == REBUILDABLE && !locked && !g.locked.is_empty() { REVIEW } else { g.category };
            return Classification::new(category, g.tag, QUARANTINE, g.cost, reason);
        }
    }

    for r in USER_RULES {
        if matches_prefix(&parts, r.prefix) {
            if r.prefix == ["Downloads"] {
                let last = parts[parts.len() - 1].to_ascii_lowercase();
                if INSTALLER_EXT.iter().any(|e| last.ends_with(&format!(".{}", e))) {
                    return Classification::new(REVIEW, "old-installer", QUARANTINE, NET_SMALL, "Installer or archive. Once the app is installed, the download can go; it can be fetched again.");
                }
            }
            return Classification::new(r.category, r.tag, r.action, r.cost, r.reason);
        }
    }
    if parts[0] == "Library" {
        return Classification::new(KEEP, "app-state", OWNER_APP, IRREPLACEABLE, "Application state, databases or settings; remove through the owning app.");
    }
    if parts[0].starts_with('.') {
        return Classification::new(KEEP, "app-state", REPORT, UNKNOWN, format!("{} tool state; preserve unless the tool is gone.", cap(parts[0].trim_start_matches('.'))));
    }
    Classification::new(REVIEW, "user-data", QUARANTINE, IRREPLACEABLE, "User or unrecognized data; size and age do not prove it is disposable.")
}

fn classify_outside(path: &str) -> Classification {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // A direct child of /Applications is user-installed software - a bundle, or a vendor folder
    // holding one - and dragging it to the Trash is the normal way to uninstall it, so it is
    // reviewable rather than protected. Anything deeper (an app's own contents) and everything
    // under /System/Applications stays protected through the rules below. Apple's own bundles
    // living here carry the SIP restricted flag and are refused at quarantine time.
    if parts.len() == 2 && parts[0] == "Applications" {
        let reason = if parts[1].ends_with(".app") {
            "Installed application. Moving the bundle to the Trash is the normal uninstall; settings and caches in ~/Library stay behind, and an app that ships a vendor uninstaller should use it."
        } else {
            "Vendor folder of installed applications. It removes as one unit, but suites like Adobe and Maxon expect their own uninstaller; check for one before moving it."
        };
        return Classification::new(REVIEW, "application", QUARANTINE, NET_LARGE, reason);
    }
    for (prefix, tag, action, cost, tool, reason) in SYSTEM_RULES {
        if matches_prefix(&parts, prefix) {
            let mut c = Classification::new(PROTECTED, tag, action, cost, *reason);
            if let Some(t) = tool {
                c.tool = Some(t.to_string());
            }
            return c;
        }
    }
    Classification::new(PROTECTED, "outside-home", REPORT, UNKNOWN, "Outside the current user home; audit only.")
}

/// Human labels for tags, used by the UI and the assistant prompt.
pub fn tag_label(tag: &str) -> &'static str {
    match tag {
        "app-cache" => "App cache",
        "browser-cache" => "Browser cache",
        "browser-profile" => "Browser profile",
        "build-output" => "Build output",
        "build-cache" => "Build cache",
        "xcode-cache" => "Xcode cache",
        "simulator-cache" => "Simulator cache",
        "simulator-device" => "Simulator devices",
        "dependency-cache" => "Dependency cache",
        "dependency-repository" => "Dependency repository",
        "managed-cleanup" => "Managed by tool",
        "managed-container-store" => "Container store",
        "virtual-machine" => "Virtual machine",
        "virtual-device" => "Virtual device",
        "unused-runtime" => "Runtime version",
        "generated-project-output" => "Generated output",
        "generated-environment" => "Environment",
        "project-cache" => "Project cache",
        "model-cache" => "Model cache",
        "downloaded-model" => "Downloaded model",
        "old-backup" => "Device backup",
        "old-installer" => "Installer",
        "release-artifact" => "Release archive",
        "test-runtime" => "Test devices",
        "diagnostic" => "Logs and reports",
        "stale-log" => "Old logs",
        "app-state" => "App state",
        "shared-app-state" => "Shared app state",
        "managed-library" => "Media library",
        "redownloadable-apple-content" => "Apple content",
        "tcc-gated" => "Private data",
        "credential" => "Credentials",
        "cloud-managed" => "Cloud synced",
        "repository" => "Repository",
        "downloads" => "Downloads",
        "user-files" => "User files",
        "user-data" => "User data",
        "orphaned-app" => "Uninstalled app",
        "orphaned-service" => "Launch agent",
        "small-abandoned" => "Small leftovers",
        "system-managed" => "System managed",
        "system-log" => "System log",
        "existing-trash" => "In Trash",
        "application" => "Application",
        "old-user-data" => "Other user",
        "privileged-helper" => "Privileged helper",
        "outside-home" => "Outside home",
        "home" => "Home",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: &str = "/Users/t";

    fn c(p: &str) -> Classification {
        classify(&format!("{}/{}", H, p), H)
    }

    #[test]
    fn jetbrains_overrides_generic_cache() {
        let j = c("Library/Caches/JetBrains/IntelliJIdea2025.1");
        assert_eq!(j.category, "review");
        assert_eq!(j.tag, "managed-cleanup");
        assert_eq!(c("Library/Caches/com.example.app").category, "rebuildable");
    }

    #[test]
    fn browser_profile_is_not_cache() {
        assert_eq!(c("Library/Application Support/Google/Chrome/Default").category, "keep");
        assert_eq!(c("Library/Caches/Google/Chrome/Default/Cache").category, "rebuildable");
    }

    #[test]
    fn docker_store_is_managed() {
        let d = c("Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw");
        assert_eq!(d.category, "keep");
        assert_eq!(d.action, "manager_command");
        assert!(d.tool.is_some());
    }

    #[test]
    fn backups_are_whole_units() {
        assert_eq!(c("Library/Application Support/MobileSync/Backup/abc123").tag, "old-backup");
        assert_eq!(c("Library/Application Support/MobileSync/Backup/abc123").category, "review");
        assert_eq!(c("Library/Application Support/MobileSync/Backup").category, "keep");
    }

    #[test]
    fn sandboxed_caches_surface() {
        assert_eq!(c("Library/Containers/com.foo.bar/Data/Library/Caches").category, "rebuildable");
        assert_eq!(c("Library/Containers/com.foo.bar/Data/Documents").category, "keep");
    }

    #[test]
    fn outside_home_is_protected_with_tags() {
        let h = classify("/opt/homebrew/Cellar", H);
        assert_eq!(h.category, "protected");
        assert_eq!(h.tag, "managed-cleanup");
        assert_eq!(classify("/private/var/folders/xx", H).tag, "system-managed");
    }

    #[test]
    fn apps_are_reviewable_but_system_apps_are_not() {
        let a = classify("/Applications/Foo.app", H);
        assert_eq!(a.category, "review");
        assert_eq!(a.tag, "application");
        assert_eq!(a.action, "quarantine");
        // Vendor folders holding an app uninstall as one unit too.
        assert_eq!(classify("/Applications/Adobe Photoshop 2026", H).category, "review");
        // The folder itself, an app's insides, and the sealed system copies stay protected.
        assert_eq!(classify("/Applications", H).category, "protected");
        assert_eq!(classify("/Applications/Foo.app/Contents/MacOS", H).category, "protected");
        assert_eq!(classify("/System/Applications/Mail.app", H).category, "protected");
    }

    #[test]
    fn installers_in_downloads() {
        assert_eq!(c("Downloads/Foo-1.2.dmg").tag, "old-installer");
        assert_eq!(c("Downloads/notes.txt").tag, "downloads");
    }

    #[test]
    fn generated_without_marker_is_review() {
        assert_eq!(c("Projects/x/node_modules").category, "review");
        assert_eq!(c("Documents/x/node_modules").tag, "generated-project-output");
        assert_eq!(c("Documents/notes.md").tag, "user-files");
    }
}
