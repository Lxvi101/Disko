# Disko

![Disko](docs/demo.gif)

A disk space app for macOS. Scan your drive, see what's eating it on a big sunburst chart, and clear it out without nuking something you needed.

I built it because my Mac kept running out of space and I had no idea where it was going. First real run freed up about **100 GB**. Now I can't live without it.

Most of it was Xcode, honestly. Simulators you forgot about, device support files for every iOS version you've ever plugged in, DerivedData from projects you deleted two years ago. Xcode is a bitch for this, so it gets its own page.

[Watch the full launch video (with sound)](docs/launch.mp4)

## Download

Grab the latest `.dmg` from [Releases](https://github.com/Lxvi101/Disko/releases/latest). It's a universal build (Apple Silicon and Intel), signed and notarized, so it opens without the usual Gatekeeper fight. From 0.2.0 on it updates itself.

## What it does

- **Quick Clean:** one click, no disk scan. In a few seconds it finds build folders (Rust `target`, `node_modules`, `.build`, venvs), package caches (npm, pnpm, uv, pip, Homebrew), Xcode DerivedData and broken simulators, Docker build cache, old installers, logs and the Trash. Results are grouped by the program that made them, anything in use is skipped, and tool-managed stores are cleaned with the tool's own command
- **Map:** scan your home folder (or the whole disk) and click through a sunburst of where the space went
- **Junk:** everything rebuildable from your disk scan, grouped by program (Rust, Node.js, Python, Xcode…) instead of one endless list
- **Suggestions:** the less obvious stuff. *AI picks* has Codex dig through your scan for forgotten projects, videos, toolchains and backups. *Duplicates* finds identical photos, videos, documents and installers (compared byte by byte, not just by name). *Leftovers* is what uninstalled apps left behind
- **Timeline:** a chart of when you last actually used your apps, projects, big files, downloads, simulators, Node/Rust/Python versions and iPhone backups. Pick a cutoff like "a year" and see what's been sitting there. For old projects it only clears the dependencies and build output, never your code
- **Apps:** laid out like the App Store, ranked by how much space each app is hiding. Xcode gets its own page for simulators, device support, DerivedData, archives and caches, broken down one by one. Also Adobe caches, old Cursor and Claude Code versions, browser caches and more
- **Quarantine:** nothing gets deleted straight away. Stuff goes to quarantine first so you can restore it if something breaks. Empty it when you're sure.
- **Agent:** optional. If you have the [Codex CLI](https://github.com/openai/codex) installed, ask it to free up space and it does the work: it scans, explains, and moves the safe stuff into quarantine itself. It can't delete anything; emptying the quarantine is always your call. Pick any model your Codex offers, or leave it on your Codex default

It's careful on purpose. Passwords, browser profiles, Photos, Mail, iCloud folders and system files are never offered for removal. Things with their own cleanup command (npm, pnpm, Homebrew, uv...) point you to that command instead of just deleting folders.

## Building it yourself

You need macOS 12+, [Rust](https://rustup.rs) and [Bun](https://bun.sh).

```sh
bun install
bun run tauri dev      # run it
bun run tauri build    # build the .app
```

Give it Full Disk Access (System Settings → Privacy & Security) if you want it to see everything. Without it, some folders like Mail and Messages just show up as unreadable.

If you're signing a build, set `APPLE_SIGNING_IDENTITY` first.

## CLI

There's also a small Python CLI in `disko_cli/` that does the scan → plan → quarantine flow from the terminal. It's handy for scripting or letting an agent drive it. No dependencies, just Python 3.11+.

```sh
./disko scan "$HOME" --output reports/home.sqlite
./disko candidates reports/home.sqlite --limit 50
./disko --help
```

See `AGENT_USAGE.md` for the full flow.

## Launch video

The video is made with [Remotion](https://remotion.dev) and lives in `video/`. The screenshots come from the real app running against a fake disk (`video/capture/`), so no real files show up.

```sh
cd video && bun install
bun run studio          # edit it live
bun run render          # full video
bun run render:readme   # the GIF at the top of this README
```

## Tests

```sh
python3 -m unittest discover -s tests
node --test tests/app-cleanup.test.ts
cd src-tauri && cargo test
```

## Things to improve

Lots. Off the top of my head:

- macOS only, and only tested on my own machine
- the rule list for what counts as junk could be way bigger
- "last used" dates come from Spotlight and file timestamps, so they're a good hint, not proof
- scanning the whole disk is slow-ish on huge drives
- the "free space" numbers are estimates, since APFS snapshots and shared blocks mess with what you actually get back
- the UI has rough edges

PRs and issues welcome.

## License

MIT
