//! Probes: targeted, deterministic, read-only checks of known locations. They run in parallel at
//! scan start and stream findings within seconds, no full walk needed.
//!
//! | probe | what |
//! |---|---|
//! | `packages` | npm, npx, pnpm (only unlinked store files), yarn, bun, cargo registry/git, Go build + module cache, Gradle, Maven, CocoaPods (cache + legacy specs repo), SwiftPM, pip, uv, Poetry, conda, Homebrew cache + old kegs (`brew cleanup`), orphaned brew deps (`brew autoremove`), electron/node-gyp/typescript download caches, Hugging Face/torch/whisper/Ollama models |
//! | `xcode` | DerivedData per project, Xcode caches, old archives per app, device support (keeps newest per platform and recently used), unavailable simulators, unused simulator runtimes, staged runtime downloads, old Xcode copies |
//! | `docker` | BuildKit cache, unused images (only when Docker/OrbStack is already running) |
//! | `versions` | nvm/fnm/volta Node, pyenv, uv Python, rustup toolchains not default or pinned, rust-docs components, JetBrains old IDE dirs, Playwright/puppeteer old browser revisions, superseded releases of Claude Code, Codex, Cursor Agent, GitKraken CLI and Ardia (their own release folders only), editor extension folders VS Code/Cursor marked obsolete |
//! | `leftovers` | data of uninstalled apps by bundle id across ~/Library, launch agents/daemons whose program is gone, empty Keystone plists, broken JDK registrations |
//! | `system` | app caches in ~/Library/Caches, logs and crash reports, Trash, installers in Downloads, staged macOS updates, iOS backups not updated for 6 months, Mail downloads, aerial wallpapers, previous Music/TV libraries, Chromium code-sign clones |
//! | `deep` | retained ZIPs verified file-by-file against their extracted folder, Adobe version payloads/nested installers/staging, duplicate compiled speech models, TeX Live old years |
//! | `patterns` | runtime pattern rules, expanded without a walk |

pub mod deep;
pub mod docker;
pub mod leftovers;
pub mod packages;
pub mod system;
pub mod versions;
pub mod xcode;

use crate::finding::make;
use crate::util::{du_all, exists, Size, MB};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use std::path::PathBuf;

/// Locations probes own, so the walk stops matching inside them (home-relative, or absolute).
pub const CLAIMS: &[(&str, Category)] = &[
    (".npm/_cacache", Category::Packages),
    (".npm/_npx", Category::Packages),
    (".npm/_logs", Category::Logs),
    ("Library/pnpm/store", Category::Packages),
    (".local/share/pnpm/store", Category::Packages),
    (".pnpm-store", Category::Packages),
    (".yarn/berry/cache", Category::Packages),
    (".bun/install/cache", Category::Packages),
    (".cargo/registry", Category::Packages),
    (".cargo/git", Category::Packages),
    ("go/pkg/mod", Category::Packages),
    (".gradle/caches", Category::Packages),
    (".gradle/wrapper/dists", Category::Packages),
    (".m2/repository", Category::Packages),
    (".cocoapods/repos", Category::Packages),
    (".cache/uv", Category::Packages),
    (".cache/pip", Category::Packages),
    (".cache/pypoetry", Category::Packages),
    (".cache/huggingface", Category::Extras),
    (".cache/torch", Category::Extras),
    (".cache/whisper", Category::Extras),
    (".cache/puppeteer", Category::OldVersions),
    (".cache/ms-playwright", Category::OldVersions),
    (".cache/go-build", Category::Packages),
    (".cache/pre-commit", Category::Packages),
    (".ollama/models", Category::Extras),
    (".nvm/versions/node", Category::OldVersions),
    (".nvm/.cache", Category::Packages),
    (".pyenv/versions", Category::OldVersions),
    (".local/share/uv/python", Category::OldVersions),
    (".rustup/toolchains", Category::OldVersions),
    (".rustup/downloads", Category::Packages),
    (".rustup/tmp", Category::Packages),
    (".codex/packages", Category::OldVersions),
    (".local/share/claude/versions", Category::OldVersions),
    (".local/share/cursor-agent/versions", Category::OldVersions),
    (".vscode/extensions", Category::OldVersions),
    (".cursor/extensions", Category::OldVersions),
    (".Trash", Category::Trash),
    ("Library/Developer/Xcode/DerivedData", Category::Developer),
    ("Library/Developer/Xcode/Archives", Category::Developer),
    ("Library/Developer/Xcode/iOS DeviceSupport", Category::Developer),
    ("Library/Developer/Xcode/watchOS DeviceSupport", Category::Developer),
    ("Library/Developer/Xcode/tvOS DeviceSupport", Category::Developer),
    ("Library/Developer/Xcode/visionOS DeviceSupport", Category::Developer),
    ("Library/Developer/Xcode/DocumentationCache", Category::Developer),
    ("Library/Developer/Xcode/iOS Device Logs", Category::Logs),
    ("Library/Developer/CoreSimulator/Caches", Category::Developer),
    ("Library/Developer/CoreSimulator/Devices", Category::Developer),
    ("Library/Developer/XCTestDevices", Category::Developer),
    ("Library/Developer/DVTDownloads", Category::Installers),
    ("Library/Application Support/MobileSync/Backup", Category::Forgotten),
    ("Library/Containers/com.apple.mail/Data/Library/Mail Downloads", Category::Cache),
    ("/Library/Updates", Category::Installers),
    ("/cores", Category::Logs),
];

fn not_apple_unless_known(name: &str) -> bool {
    !name.starts_with("com.apple.") || system::APPLE_CACHES_OK.contains(&name)
}

fn any(_: &str) -> bool {
    true
}

/// Folders whose children probes own.
pub const CLAIM_CHILDREN: &[(&str, Category, fn(&str) -> bool)] = &[
    ("Library/Caches", Category::Cache, not_apple_unless_known),
    ("Library/Logs", Category::Logs, any),
    (".cache", Category::Cache, any),
];

/// One finding for a fixed set of paths (home-relative or absolute), if they exist and are big
/// enough. Measured with `du`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fixed(c: &Clutter, rule: &str, cat: Category, title: &str, icon: &str, rels: &[&str], safety: Safety, regrows: bool, why: &str, min: u64) -> Option<Finding> {
    let paths: Vec<PathBuf> = rels.iter().map(|r| if r.starts_with('/') { c.env.sys(r) } else { c.env.home.join(r) }).filter(|p| exists(p)).collect();
    if paths.is_empty() {
        return None;
    }
    let size = du_all(&paths);
    if size.bytes < min {
        return None;
    }
    Some(make(rule, cat, title, icon, paths.iter().map(|p| p.to_string_lossy().to_string()).collect(), size, safety, regrows, why))
}

pub(crate) const MIN: u64 = MB;

/// One probe: a name and a function.
pub type Probe = (&'static str, fn(&Clutter) -> Vec<Finding>);

pub const ALL: &[Probe] = &[
    ("packages", packages::probe),
    ("homebrew", packages::homebrew),
    ("models", packages::models),
    ("xcode", xcode::probe),
    ("simulators", xcode::simulators),
    ("docker", docker::probe),
    ("versions", versions::probe),
    ("agent-releases", versions::releases),
    ("editor-extensions", versions::extensions),
    ("leftovers", leftovers::probe),
    ("launch-items", leftovers::launch_items),
    ("caches", system::caches),
    ("logs", system::logs),
    ("trash", system::trash),
    ("system", system::probe),
    ("downloads", system::downloads),
    ("archives", deep::archives),
    ("adobe", deep::adobe),
    ("speech", deep::speech),
    ("texlive", deep::texlive),
    ("patterns", patterns),
];

pub fn run_all(c: &Clutter, sink: &(dyn Fn(Vec<Finding>) + Sync)) {
    // Warm the shared inventories once instead of racing to build them in every probe.
    rayon::join(|| c.apps(), || c.procs());
    rayon::scope(|s| {
        for (_, probe) in ALL {
            s.spawn(move |_| {
                let found = probe(c);
                if !found.is_empty() {
                    sink(found);
                }
            });
        }
    });
}

/// Runtime pattern rules, expanded along their literal prefix.
fn patterns(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    for (r, g) in c.pattern_rules() {
        for p in g.expand(500) {
            let size: Size = crate::util::du(&p);
            out.push(c.pattern_match(r, &p.to_string_lossy(), size));
        }
    }
    out
}
