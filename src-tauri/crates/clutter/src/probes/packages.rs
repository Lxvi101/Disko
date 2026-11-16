//! Package manager stores and downloads, preferring the owning tool's own cleanup command.

use super::{fixed, MIN};
use crate::finding::{make, FindingExt};
use crate::procs::who;
use crate::util::{child_dirs, du, du_all, du_unlinked, exists, human, mtime, name_of, now, parse_version, run, s, tool, version_str, Size, DAY};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

fn busy(c: &Clutter, names: &[&str]) -> Option<String> {
    c.procs().named(names).map(|p| format!("{} is running", who(p)))
}

struct Store {
    rule: &'static str,
    title: &'static str,
    icon: &'static str,
    paths: &'static [&'static str],
    why: &'static str,
    busy: &'static [&'static str],
    category: Category,
}

const STORES: &[Store] = &[
    Store { rule: "packages.npm", title: "npm", icon: "node", paths: &[".npm/_cacache"], why: "Every package version npm ever downloaded; npm fetches what it needs again.", busy: &["npm"], category: Category::Packages },
    Store { rule: "packages.yarn", title: "Yarn", icon: "node", paths: &["Library/Caches/Yarn", ".yarn/berry/cache", ".cache/yarn"], why: "Yarn's download cache; packages download again when a project installs them.", busy: &["yarn"], category: Category::Packages },
    Store { rule: "packages.bun", title: "Bun", icon: "node", paths: &[".bun/install/cache", "Library/Caches/bun"], why: "Bun's global install cache; refilled on the next bun install.", busy: &["bun"], category: Category::Packages },
    Store { rule: "packages.cargo", title: "Cargo crates", icon: "rust", paths: &[".cargo/registry", ".cargo/git"], why: "Downloaded crates, their unpacked sources and git checkouts; cargo fetches them again on the next build. Installed tools stay.", busy: &["cargo", "rustc", "rust-analyzer"], category: Category::Packages },
    Store { rule: "packages.gradle", title: "Gradle", icon: "java", paths: &[".gradle/caches"], why: "Downloaded dependencies and build caches; Gradle downloads them again on the next sync.", busy: &["java", "gradle"], category: Category::Packages },
    Store { rule: "packages.cocoapods", title: "CocoaPods", icon: "xcode", paths: &["Library/Caches/CocoaPods", ".cocoapods/repos/trunk"], why: "Downloaded pod specs and sources; fetched again on the next pod install.", busy: &["pod"], category: Category::Packages },
    Store { rule: "packages.cocoapods-master", title: "CocoaPods specs (old)", icon: "xcode", paths: &[".cocoapods/repos/master"], why: "A full git checkout of the old Specs repo. CocoaPods uses its CDN since 1.8, so this is unused unless a Podfile names it as a source.", busy: &["pod"], category: Category::Packages },
    Store { rule: "packages.swiftpm", title: "Swift packages", icon: "swift", paths: &["Library/Caches/org.swift.swiftpm"], why: "Repositories SwiftPM cloned to resolve packages; cloned again on the next resolve.", busy: &["swift-build", "swift-package", "xcodebuild"], category: Category::Packages },
    Store { rule: "packages.pip", title: "pip", icon: "python", paths: &["Library/Caches/pip", ".cache/pip"], why: "Downloaded wheels and HTTP responses; pip fetches what it needs again.", busy: &["pip", "pip3"], category: Category::Packages },
    Store { rule: "packages.poetry", title: "Poetry", icon: "python", paths: &["Library/Caches/pypoetry", ".cache/pypoetry"], why: "Poetry's package and artifact cache; refilled on the next install.", busy: &["poetry"], category: Category::Packages },
    Store { rule: "packages.build-tools", title: "Build tool downloads", icon: "node", paths: &["Library/Caches/electron", "Library/Caches/electron-builder", "Library/Caches/node-gyp", "Library/Caches/typescript", ".node-gyp", ".cache/node-gyp", ".cache/typescript"], why: "Electron, node-gyp and TypeScript download caches; fetched again on the next build.", busy: &[], category: Category::Packages },
    Store { rule: "packages.deno", title: "Deno", icon: "node", paths: &["Library/Caches/deno"], why: "Deno's module cache; downloaded again when a script needs it.", busy: &["deno"], category: Category::Packages },
    Store { rule: "packages.nuget", title: "NuGet", icon: "box", paths: &[".nuget/packages", "Library/Caches/NuGet"], why: "NuGet packages; restored again by the next dotnet build.", busy: &["dotnet"], category: Category::Packages },
    Store { rule: "packages.composer", title: "Composer", icon: "box", paths: &["Library/Caches/composer", ".composer/cache"], why: "Composer's download cache; refilled on the next install.", busy: &["composer"], category: Category::Packages },
    Store { rule: "packages.pre-commit", title: "pre-commit", icon: "box", paths: &[".cache/pre-commit"], why: "Hook environments pre-commit builds; rebuilt the next time hooks run.", busy: &["pre-commit"], category: Category::Packages },
    Store { rule: "packages.rustup-downloads", title: "rustup downloads", icon: "rust", paths: &[".rustup/downloads", ".rustup/tmp"], why: "Leftover rustup downloads and temp files.", busy: &["rustup"], category: Category::Packages },
    Store { rule: "packages.nvm-cache", title: "nvm downloads", icon: "node", paths: &[".nvm/.cache"], why: "Node tarballs nvm downloaded to install versions.", busy: &[], category: Category::Packages },
    Store { rule: "packages.sccache", title: "sccache", icon: "rust", paths: &["Library/Caches/Mozilla.sccache"], why: "Shared compiler cache; refilled by the next builds.", busy: &["sccache"], category: Category::Packages },
    Store { rule: "logs.npm", title: "npm logs", icon: "log", paths: &[".npm/_logs"], why: "Debug logs npm writes for every run.", busy: &[], category: Category::Logs },
];

/// Folder names in ~/Library/Caches this probe already covers.
pub const HANDLED_CACHES: &[&str] = &[
    "Yarn", "bun", "CocoaPods", "org.swift.swiftpm", "pip", "pypoetry", "electron", "electron-builder", "node-gyp", "typescript", "deno", "NuGet", "composer", "Mozilla.sccache", "go-build",
    "Homebrew", "ms-playwright", "Cypress", "JetBrains", "com.apple.dt.Xcode", "com.apple.dt.xcodebuild", "pnpm",
];

pub fn probe(c: &Clutter) -> Vec<Finding> {
    let mut out: Vec<Finding> = STORES
        .par_iter()
        .filter_map(|st| {
            let f = fixed(c, st.rule, st.category, st.title, st.icon, st.paths, Safety::Safe, true, st.why, MIN)?;
            Some(f.busy(if st.busy.is_empty() { None } else { busy(c, st.busy) }))
        })
        .collect();
    let jobs: Vec<fn(&Clutter) -> Vec<Finding>> = vec![npx, pnpm, uv, go, gradle_dists, maven, conda, home_cache];
    out.extend(jobs.par_iter().flat_map(|j| j(c)).collect::<Vec<_>>());
    out
}

/// npx installs: only ones nothing used for a month (MCP servers often run from here).
fn npx(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join(".npm/_npx");
    let old: Vec<PathBuf> = child_dirs(&root).into_iter().filter(|p| crate::util::shallow_newest(p).is_some_and(|t| now() - t > 30 * DAY)).collect();
    if old.is_empty() {
        return vec![];
    }
    let procs = c.procs();
    let old: Vec<PathBuf> = old.into_iter().filter(|p| procs.exe_inside(p).is_none() && procs.cwd_inside(p).is_none()).collect();
    let size = du_all(&old);
    if size.bytes < MIN {
        return vec![];
    }
    let n = old.len();
    vec![make("packages.npx", Category::Packages, "npx", "node", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, format!("{} packages npx installed for one-off runs and nothing used for a month; npx downloads them again if needed. Recent ones stay.", n))
        .keyed(&s(&root))
        .detail(format!("{} unused installs", n))]
}

/// pnpm's store is hard-linked into projects: only files no project links free space. Each pnpm
/// major keeps its own store format (v3, v10, v11 …); formats older than the one in use are dead.
fn pnpm(c: &Clutter) -> Vec<Finding> {
    let Some(bin) = tool(&c.env.home, "pnpm") else { return vec![] };
    let current = if c.env.tools { run(&bin, &["store", "path"], &c.env.home, std::time::Duration::from_secs(15)).map(|o| PathBuf::from(o.trim())) } else { None };
    let mut out = vec![];
    for rel in ["Library/pnpm/store", ".local/share/pnpm/store", ".pnpm-store"] {
        let root = c.env.home.join(rel);
        if !exists(&root) {
            continue;
        }
        let versions: Vec<PathBuf> = child_dirs(&root).into_iter().filter(|p| name_of(p).strip_prefix('v').is_some_and(|n| n.parse::<u32>().is_ok())).collect();
        let cur = current.clone().filter(|c| versions.contains(c)).or_else(|| versions.iter().max_by_key(|p| name_of(p)[1..].parse::<u32>().unwrap_or(0)).cloned());
        for v in &versions {
            let total = du(v);
            let unlinked = du_unlinked(v);
            if Some(v) == cur.as_ref() {
                if unlinked.bytes < MIN {
                    continue;
                }
                let why = format!("`pnpm store prune` removes packages no project references. {} of the {} store isn't hard-linked anywhere, but on APFS pnpm usually clones files into projects, so the real saving can be smaller.", human(unlinked.bytes), human(total.bytes));
                out.push(
                    make("packages.pnpm", Category::Packages, "pnpm", "node", vec![s(v)], Size { bytes: unlinked.bytes, ..total }, Safety::Safe, true, why)
                        .command(&[&s(&bin), "store", "prune"], "pnpm store prune")
                        .busy(busy(c, &["pnpm"])),
                );
            } else if total.bytes >= MIN {
                let curname = cur.as_ref().map(|p| name_of(p)).unwrap_or_default();
                let why = format!(
                    "The store format an older pnpm used; this pnpm uses {}. Projects installed with it keep working (their files are links or APFS clones), so up to {} of {} comes back now and the rest as those projects reinstall.",
                    curname,
                    human(unlinked.bytes),
                    human(total.bytes)
                );
                out.push(make("versions.pnpm-store", Category::OldVersions, format!("pnpm store {}", name_of(v)), "node", vec![s(v)], Size { bytes: unlinked.bytes, ..total }, Safety::Safe, false, why).busy(busy(c, &["pnpm"])));
            }
        }
        // Stores from before the versioned layout.
        if versions.is_empty() {
            let total = du(&root);
            let unlinked = du_unlinked(&root);
            if unlinked.bytes >= MIN {
                out.push(
                    make("packages.pnpm", Category::Packages, "pnpm", "node", vec![s(&root)], Size { bytes: unlinked.bytes, ..total }, Safety::Safe, true, format!("{} of the store is linked by no project; `pnpm store prune` removes only those.", human(unlinked.bytes)))
                        .command(&[&s(&bin), "store", "prune"], "pnpm store prune")
                        .busy(busy(c, &["pnpm"])),
                );
            }
        }
    }
    out
}

fn uv(c: &Clutter) -> Vec<Finding> {
    let p = c.env.home.join(".cache/uv");
    if !exists(&p) {
        return vec![];
    }
    let size = du(&p);
    if size.bytes < MIN {
        return vec![];
    }
    let mut f = make("packages.uv", Category::Packages, "uv", "python", vec![s(&p)], size, Safety::Safe, true, "Wheels, sources and builds uv cached; environments keep their own copies and uv downloads again when needed.");
    if let Some(bin) = tool(&c.env.home, "uv") {
        f = f.command(&[&s(&bin), "cache", "clean"], "uv cache clean");
    }
    vec![f.busy(busy(c, &["uv"]))]
}

fn go(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    let bin = tool(&c.env.home, "go");
    for (rel, rule, title, why, args) in [
        ("Library/Caches/go-build", "packages.go-build", "Go build cache", "Compiled Go packages; rebuilt on the next go build.", ["clean", "-cache"]),
        ("go/pkg/mod", "packages.go-mod", "Go modules", "Downloaded Go modules; fetched again on the next build.", ["clean", "-modcache"]),
    ] {
        let p = c.env.home.join(rel);
        if !exists(&p) {
            continue;
        }
        let size = du(&p);
        if size.bytes < MIN {
            continue;
        }
        let mut f = make(rule, Category::Packages, title, "go", vec![s(&p)], size, Safety::Safe, true, why);
        match &bin {
            Some(b) => f = f.command(&[&s(b), args[0], args[1]], &format!("go {} {}", args[0], args[1])),
            // Module files are read-only; without go we can't remove them cleanly.
            None if rule == "packages.go-mod" => continue,
            None => {}
        }
        out.push(f.busy(busy(c, &["go"])));
    }
    out
}

/// Gradle keeps every wrapper distribution it ever downloaded; keep the newest.
fn gradle_dists(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join(".gradle/wrapper/dists");
    let mut v: Vec<(Vec<u64>, PathBuf)> = child_dirs(&root).into_iter().filter_map(|p| Some((parse_version(name_of(&p).strip_prefix("gradle-")?)?, p))).collect();
    if v.len() < 2 {
        return vec![];
    }
    v.sort();
    let newest = version_str(&v.last().unwrap().0);
    let old: Vec<PathBuf> = v[..v.len() - 1].iter().map(|x| x.1.clone()).collect();
    let size = du_all(&old);
    if size.bytes < MIN {
        return vec![];
    }
    let names: Vec<String> = v[..v.len() - 1].iter().map(|x| version_str(&x.0)).collect();
    vec![make("versions.gradle", Category::OldVersions, "Gradle", "java", old.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, format!("Gradle {} is the newest downloaded; projects still on an older wrapper download theirs again.", newest))
        .keyed(&s(&root))
        .detail(names.join(", "))
        .busy(busy(c, &["java", "gradle"]))]
}

fn maven(c: &Clutter) -> Vec<Finding> {
    fixed(c, "packages.maven", Category::Packages, "Maven", "java", &[".m2/repository"], Safety::Review, true, "Maven's local repository. Mostly downloads, but artifacts you installed locally exist nowhere else.", 10 * MIN)
        .map(|f| f.busy(busy(c, &["java", "mvn"])))
        .into_iter()
        .collect()
}

fn conda(c: &Clutter) -> Vec<Finding> {
    let mut out = vec![];
    for base in ["miniconda3", "anaconda3", "miniforge3", "mambaforge", "opt/miniconda3"] {
        let b = c.env.home.join(base);
        let pkgs = b.join("pkgs");
        let bin = b.join("bin/conda");
        if !exists(&pkgs) || !bin.is_file() {
            continue;
        }
        let size = du(&pkgs);
        if size.bytes < MIN {
            continue;
        }
        out.push(
            make("packages.conda", Category::Packages, "Conda", "python", vec![s(&pkgs)], size, Safety::Safe, true, "Package tarballs and extracted packages conda keeps after installing; `conda clean --all` removes what no environment uses.")
                .command(&[&s(&bin), "clean", "--all", "-y"], "conda clean --all")
                .busy(busy(c, &["conda", "python"])),
        );
    }
    out
}

/// Children of ~/.cache (XDG cache dir) no other rule covers. Apps must cope with losing them,
/// but some tools keep venvs or runtimes here, so they go to the Trash.
const KNOWN_DOT_CACHE: &[&str] = &["uv", "pip", "pypoetry", "huggingface", "torch", "whisper", "puppeteer", "ms-playwright", "go-build", "pre-commit", "yarn", "node-gyp", "typescript", "bazel"];

fn home_cache(c: &Clutter) -> Vec<Finding> {
    let root = c.env.home.join(".cache");
    let procs = c.procs();
    child_dirs(&root)
        .into_par_iter()
        .filter(|p| !KNOWN_DOT_CACHE.contains(&name_of(p).as_str()))
        .flat_map_iter(|p| {
            let name = name_of(&p);
            let busy = procs.exe_inside(&p).or_else(|| procs.cwd_inside(&p)).map(|x| format!("{} is running from it", who(x)));
            // Build output parked here (CARGO_TARGET_DIR and friends) is rebuildable: split it out.
            let (targets, rest): (Vec<PathBuf>, Vec<PathBuf>) = crate::util::children(&p).into_iter().partition(|e| crate::util::is_dir(e) && std::fs::read_to_string(e.join("CACHEDIR.TAG")).is_ok_and(|t| t.contains("Signature: 8a477f597d28d172789f06886806bc55")));
            let mut out = vec![];
            if !targets.is_empty() {
                let size = du_all(&targets);
                if size.bytes >= MIN {
                    let busy_build = busy.clone().or_else(|| procs.tool_in(&p, &["cargo", "rustc"]).map(|x| format!("{} is running", who(x))));
                    let mut f = make("build", Category::Build, crate::group::project_title(&p), "rust", vec![], size, Safety::Safe, true, format!("Build output (tagged as a cache directory) kept outside its project; the next build makes it again. Last changed {}.", crate::util::ago(size.newest)))
                        .keyed(&s(&p))
                        .detail(format!("Cargo build · {} folders", targets.len()))
                        .busy(busy_build);
                    f.paths = targets.iter().map(|t| s(t)).collect();
                    out.push(f);
                }
            }
            let size = du_all(&rest);
            if size.bytes >= MIN {
                let mut f = make("cache.dot-cache", Category::Cache, crate::util::cap(&name), "box", vec![], size, Safety::Review, true, format!("A tool's cache folder in ~/.cache, last written {}; tools rebuild what they need, though some keep environments here.", crate::util::ago(size.newest)))
                    .keyed(&s(&p))
                    .detail("~/.cache")
                    .busy(busy);
                // The whole folder when nothing was split out, else what's left of it.
                f.paths = if targets.is_empty() { vec![s(&p)] } else { rest.iter().map(|r| s(r)).collect() };
                out.push(f);
            }
            out
        })
        .collect()
}

/// Homebrew: old kegs and downloads (`brew cleanup`), and dependencies nothing needs any more
/// (`brew autoremove`), measured from Homebrew's own dry runs.
pub fn homebrew(c: &Clutter) -> Vec<Finding> {
    if !c.env.tools {
        return vec![];
    }
    let Some(brew) = tool(&c.env.home, "brew") else { return vec![] };
    let home = &c.env.home;
    let mut out = vec![];
    let busy_brew = busy(c, &["brew"]);
    let cleanup = run(&brew, &["cleanup", "-n", "--prune=all", "-s"], home, Duration::from_secs(60)).unwrap_or_default();
    let (mut kegs, mut downloads) = (vec![], vec![]);
    for line in cleanup.lines() {
        let Some(rest) = line.strip_prefix("Would remove: ") else { continue };
        let path = rest.rsplit_once(" (").map(|(p, _)| p).unwrap_or(rest);
        let p = PathBuf::from(path);
        if !exists(&p) {
            continue;
        }
        if path.contains("/Cellar/") || path.contains("/Caskroom/") || path.contains("portable-ruby") {
            kegs.push(p);
        } else {
            downloads.push(p);
        }
    }
    let cache = home.join("Library/Caches/Homebrew");
    if exists(&cache) && !downloads.iter().any(|d| d == &cache) {
        // `-s` scrubs the whole download cache, including bottles of current versions.
        downloads.retain(|d| !d.starts_with(&cache));
        downloads.push(cache);
    }
    let argv = [s(&brew), "cleanup".into(), "--prune=all".into(), "-s".into()];
    let argv: Vec<&str> = argv.iter().map(|x| x.as_str()).collect();
    if !kegs.is_empty() {
        let size = du_all(&kegs);
        if size.bytes >= MIN {
            let names: Vec<String> = kegs.iter().map(|k| format!("{} {}", k.parent().map(name_of).unwrap_or_default(), name_of(k))).collect();
            out.push(
                make("versions.homebrew", Category::OldVersions, "Homebrew", "brew", kegs.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, format!("{} older formula versions next to the ones in use; Homebrew's own cleanup removes them.", kegs.len()))
                    .keyed("homebrew-kegs")
                    .detail(names.join(", "))
                    .command(&argv, "brew cleanup")
                    .busy(busy_brew.clone()),
            );
        }
    }
    if !downloads.is_empty() {
        let size = du_all(&downloads);
        if size.bytes >= MIN {
            out.push(
                make("packages.homebrew", Category::Packages, "Homebrew downloads", "brew", downloads.iter().map(|p| s(p)).collect(), size, Safety::Safe, true, "Bottles and source archives Homebrew kept after installing; it downloads them again if it ever needs to reinstall.")
                    .keyed("homebrew-downloads")
                    .command(&argv, "brew cleanup")
                    .busy(busy_brew.clone()),
            );
        }
    }
    // Dependencies whose dependents are gone.
    let auto = run(&brew, &["autoremove", "-n"], home, Duration::from_secs(60)).unwrap_or_default();
    let cellar = brew.parent().and_then(|b| b.parent()).map(|p| p.join("Cellar")).unwrap_or_default();
    let orphans: Vec<String> = auto.lines().filter(|l| !l.starts_with("==>") && !l.trim().is_empty() && !l.contains(' ')).map(|l| l.trim().to_string()).collect();
    let orphan_paths: Vec<PathBuf> = orphans.iter().map(|n| cellar.join(n)).filter(|p| exists(p)).collect();
    if !orphan_paths.is_empty() {
        let size = du_all(&orphan_paths);
        out.push(
            make("leftovers.homebrew-deps", Category::Leftovers, "Homebrew dependencies", "brew", orphan_paths.iter().map(|p| s(p)).collect(), size, Safety::Safe, false, "Installed only as dependencies of formulae you've since removed; nothing needs them now.")
                .keyed("homebrew-autoremove")
                .detail(orphans.join(", "))
                .command(&[&s(&brew), "autoremove"], "brew autoremove")
                .busy(busy_brew.clone()),
        );
    }
    // Versioned toolchains nothing depends on, when a newer one of the same family is installed.
    let leaves = run(&brew, &["leaves", "-r"], home, Duration::from_secs(30)).unwrap_or_default();
    let installed: Vec<String> = crate::util::children(&cellar).iter().map(|p| name_of(p)).collect();
    for leaf in leaves.lines().map(str::trim).filter(|l| l.contains('@')) {
        let Some((base, ver)) = leaf.split_once('@') else { continue };
        let Some(v) = parse_version(ver) else { continue };
        let newer = installed.iter().filter(|n| *n != leaf).find(|n| {
            n.as_str() == base || n.split_once('@').is_some_and(|(b, nv)| b == base && parse_version(nv).is_some_and(|nv| nv > v))
        });
        let Some(newer) = newer else { continue };
        let p = cellar.join(leaf);
        let size = du(&p);
        if size.bytes < MIN {
            continue;
        }
        out.push(
            make("versions.homebrew-toolchain", Category::OldVersions, leaf, "brew", vec![s(&p)], size, Safety::Review, false, format!("You installed {} yourself and nothing depends on it; {} is installed too.", leaf, newer))
                .detail("Homebrew formula")
                .command(&[&s(&brew), "uninstall", leaf], &format!("brew uninstall {}", leaf))
                .busy(busy_brew.clone()),
        );
    }
    out
}

/// Downloaded AI models: Hugging Face, torch hub, Whisper, Ollama. Optional content (Extras).
pub fn models(c: &Clutter) -> Vec<Finding> {
    let home = &c.env.home;
    let mut out = vec![];
    for hub in [home.join(".cache/huggingface/hub"), home.join(".cache/huggingface/datasets")] {
        for p in child_dirs(&hub) {
            let n = name_of(&p);
            let Some(rest) = n.strip_prefix("models--").or_else(|| n.strip_prefix("datasets--")) else { continue };
            let size = du(&p);
            if size.bytes < 50 * MIN {
                continue;
            }
            let title = rest.replacen("--", "/", 1);
            out.push(
                make("extras.huggingface", Category::Extras, title, "box", vec![s(&p)], size, Safety::Review, false, format!("A Hugging Face download last used {}; it downloads again if code asks for it.", crate::util::ago(size.newest)))
                    .detail("Hugging Face model"),
            );
        }
    }
    for (rel, title) in [(".cache/torch", "PyTorch hub"), (".cache/whisper", "Whisper models"), (".cache/huggingface/xet", "Hugging Face chunks")] {
        if let Some(f) = fixed(c, "extras.models", Category::Extras, title, "box", &[rel], Safety::Review, false, "Downloaded model weights; fetched again on next use.", 50 * MIN) {
            out.push(f);
        }
    }
    // Ollama: one finding per model, removed with `ollama rm`.
    if let Some(bin) = tool(home, "ollama") {
        let manifests = home.join(".ollama/models/manifests/registry.ollama.ai/library");
        for model in child_dirs(&manifests) {
            for tag in crate::util::children(&model) {
                let name = format!("{}:{}", name_of(&model), name_of(&tag));
                let Ok(text) = std::fs::read_to_string(&tag) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                let bytes: u64 = v["layers"].as_array().into_iter().flatten().filter_map(|l| l["size"].as_u64()).sum();
                if bytes < 50 * MIN {
                    continue;
                }
                out.push(
                    make("extras.ollama", Category::Extras, name.clone(), "box", vec![s(&tag)], Size { bytes, files: 1, newest: mtime(&tag) }, Safety::Review, false, format!("An Ollama model pulled {}; `ollama pull` gets it back.", crate::util::ago(mtime(&tag))))
                        .detail("Ollama model")
                        .command(&[&s(&bin), "rm", &name], &format!("ollama rm {}", name)),
                );
            }
        }
    }
    out
}
