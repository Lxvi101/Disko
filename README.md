# Disko

A disk space app for macOS. Scan your drive, see what's eating it on a big sunburst chart, and clear it out without nuking something you needed.

I built it because my Mac kept running out of space and I had no idea where it was going. First real run freed up about **100 GB**. Now I can't live without it.

Most of it was Xcode, honestly. Simulators you forgot about, device support files for every iOS version you've ever plugged in, DerivedData from projects you deleted two years ago. Xcode is a bitch for this, so it gets its own page.

## What it does

- **Explore:** scan your home folder (or the whole disk) and click through a sunburst of where the space went
- **Suggestions:** caches, build output and other junk that gets rebuilt anyway, with a reason for every item
- **Inactive:** big files you haven't touched in months
- **Xcode:** simulators, device support, DerivedData, archives and caches, broken down one by one
- **App cleanup:** leftovers from apps you uninstalled, plus old versions some apps leave lying around (Adobe, Cursor, Claude Code, etc.)
- **Quarantine:** nothing gets deleted straight away. Stuff goes to quarantine first so you can restore it if something breaks. Empty it when you're sure.
- **Assistant:** optional. If you have the [Codex CLI](https://github.com/openai/codex) installed, it can read your scan and explain what's worth removing. It's read-only and can't delete anything itself.

It's careful on purpose. Passwords, browser profiles, Photos, Mail, iCloud folders and system files are never offered for removal. Things with their own cleanup command (npm, pnpm, Homebrew, uv...) point you to that command instead of just deleting folders.

## Running it

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

## Tests

```sh
python3 -m unittest discover -s tests
cd src-tauri && cargo test
```

## Things to improve

Lots. Off the top of my head:

- macOS only, and only tested on my own machine
- no signed or notarized release yet, so you have to build it yourself
- the rule list for what counts as junk could be way bigger
- scanning the whole disk is slow-ish on huge drives
- the "free space" numbers are estimates, since APFS snapshots and shared blocks mess with what you actually get back
- the UI has rough edges

PRs and issues welcome.

## License

MIT
