# Disko

**A factory-fresh Mac, always.**

Disko finds everything that piled up since your Mac left the factory: build output, caches,
package downloads, old versions of tools next to the ones you use, leftovers of apps you deleted,
installers that already did their job, coding-agent histories. Then it gets rid of it.

- **One scan, automatic.** Open Disko and it's already looking. The whole disk, live: the map
  grows while it scans, and clutter shows up within a second.
- **Gone means gone.** Remove something and the map updates instantly. Rebuildable stuff is
  deleted in one tap; anything that needs your judgment goes to the Trash with Undo. No pop-ups.
- **See where the space went.** Sunburst, bubbles, blocks or a list. Even the space a file
  scan can't see (snapshots, macOS volumes, swap) is explained.
- **Deep, not superficial.** Old Node, Python and Rust toolchains nothing uses, superseded
  Claude Code and Codex releases, obsolete editor extensions, ZIPs already unpacked next to
  themselves, Xcode device symbols for iOS versions you no longer run, simulator runtimes,
  Homebrew leftovers, Adobe payloads, dead launch agents…
- **Apps and agents.** Every app with all its data. Every Claude Code, Codex and T3 Code
  project and session with its size and date.
- **Memory.** Tell Disko what to keep and what to always clean ("build files every 2 days").
- **Built for agents.** Disko is an MCP server: your coding agent can ask where the space went,
  explain any path, clean, and teach Disko new kinds of clutter. Connect it from the Memory tab.

It's careful on purpose: system files, your documents, photos, mail, iCloud, keychains and
anything on your keep list are never touched, and tools with their own cleanup command
(Homebrew, npm, uv, Xcode simulators…) are cleaned with that command.

## Download

Get the latest `.dmg` from [Releases](https://github.com/Lxvi101/Disko/releases/latest).
Universal, signed and notarized, and it updates itself. Give it Full Disk Access to see everything.

## Build

macOS 12+, [Rust](https://rustup.rs) and [Bun](https://bun.sh).

```sh
bun install
bun run tauri dev                       # run it
cd src-tauri && cargo test --workspace  # tests
```

`Disko --mcp` runs the MCP server; `Disko --bench` prints a full scan summary as JSON.
Where things live: [AGENTS.md](AGENTS.md).

## License

MIT
