# Disko

**Ethos: a factory-fresh Mac, always.** Find what accumulated since the Mac left the factory
(build output, caches, leftovers of removed apps, superseded versions, installers, agent histories)
and get rid of it. One automatic scan, live results, instant removals, no pop-ups. Visual over
verbose: icons and big numbers, fine print only where it's truly needed. Protect the user's work,
credentials and explicit keep choices. Prefer the owning tool's cleanup command.
Deterministic, evidence-backed rules over guesses.

## Where things are

| Path | What |
|---|---|
| `src-tauri/crates/core` | Shared types (`Finding`, `TreeNode` …) and the only remove/trash ops, with the protected-path guard |
| `src-tauri/crates/engine` | The one whole-disk scan, in-memory tree, snapshot, FSEvents live updates, hidden-space accounting |
| `src-tauri/crates/clutter` | What counts as clutter: walk rules + probes → findings, and cleaning |
| `src-tauri/crates/apps` | Apps with their data and leftovers; coding-agent sessions |
| `src-tauri/crates/memory` | Keep list, learned rules, routines, community rule pack |
| `src-tauri/crates/mcp` | MCP server for agents (`Disko --mcp`) |
| `src-tauri/src` | Tauri app: commands and events wiring the crates |
| `src` | React UI (`src/README.md`) |
| `rules/` | Community rule pack |
| `docs/contract.md` | Commands, events and data shapes between UI and app |

## Working here

- `cd src-tauri && cargo test --workspace -j 4` · `bun run build` · `bun run tauri dev`
- Release: bump the version in `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`;
  the updater key and endpoint in `tauri.conf.json` must never change.
- New kinds of clutter go in `crates/clutter` as rules with a fixture test.
