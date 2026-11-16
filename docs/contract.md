# Disko 1.0 build contract

Disko 1.0 is a rebuild. Ethos: **get this Mac back as close as possible to how it was when it left
the factory.** One automatic scan, live results, deep deterministic findings, instant removals,
no pop-ups, visual over verbose. Agents use Disko through an MCP server, not a chat sidebar.

Old code (0.6) lives in `legacy/` as read-only reference. Port ideas and good code from it; do not
port its UX (many scan modes, quarantine flow, modal confirmations, label spam).

## Layout

| Path | Owner | What |
|---|---|---|
| `src-tauri/crates/core` | lead | Shared types (`Finding`, `TreeNode`, `VolumeInfo`, `WalkMatcher` …) and the only remove/trash ops. |
| `src-tauri/crates/engine` | engine agent | One whole-disk scan, in-memory tree, snapshot, FSEvents live updates, volume accounting. |
| `src-tauri/crates/clutter` | clutter agent | Rules (data-driven) + probes (targeted deterministic checks) → `Finding`s; cleaning. |
| `src-tauri/crates/apps` | apps agent | Installed apps with their data + leftovers, uninstall; coding-agent sessions per tool/project/session. |
| `src-tauri/crates/memory` | memory agent | Keep list, learned rules, routines, community rule pack; MCP server (`disko --mcp`). |
| `src-tauri/src` | lead | Tauri app: commands + events wiring the crates together. CLI entry points. |
| `src` | frontend agent | React UI. |

Each crate depends only on `disko-core` (+ external crates), except `memory`'s MCP module, which
uses all crates. Build and test your crate alone: `cargo test -p disko-<name> -j 4` from
`src-tauri/`. **This Mac has 16 GB RAM: always `-j 4`, and wrap cargo in
`flock /tmp/fd-build.lock cargo …` so only one build runs at a time.** Do not edit another
owner's directory; ask the lead (in your final report) for core changes you need.

Disko's own footprint: one snapshot in `~/Library/Caches/app.disko` (target < 50 MB for a
full disk), settings/memory in `~/Library/Application Support/app.disko`. Nothing else.

## Removal model (no quarantine, no modals)

- `Safety::Safe` (rebuildable, regrows): `Action::Delete` or a tool `Command`. One tap.
- `Safety::Review`: `Action::Trash` (macOS Trash, Undo via `ops::put_back`). One tap, Undo toast.
- Root-owned: `admin: true` → one macOS authorization prompt for the batch (`ops::delete_as_admin`).
- Keep list always wins. In-use paths (`busy`) are skipped.
- Agents (MCP) never delete permanently: findings go to the Trash or run the owning tool's command.
  Rules from agents and the community pack are always `Review` and never run in routines.
- After any removal the engine drops those paths from its tree immediately and the UI updates
  without a rescan. Changes made outside Disko arrive through FSEvents within ~1 s.

## Tauri commands (frontend ↔ app)

All return JSON matching `crates/core/src/types.rs` (camelCase is NOT used; fields are snake_case).

| Command | Args | Returns |
|---|---|---|
| `boot` | – | `{ version, home, full_disk_access: bool, volume: VolumeInfo, snapshot_age_s: number\|null }` and starts the scan automatically |
| `rescan` | – | `null` (restarts the one scan) |
| `tree` | `{ path: string\|null, depth: number }` | `TreeNode` (null path = root "/") |
| `search` | `{ query: string, limit?: number }` | `TreeNode[]` (flat, no children) |
| `clutter` | – | `Finding[]` (current set) |
| `clean` | `{ ids: string[], paths: string[] }` | `CleanResult` |
| `undo` | `{ trashed: Trashed[] }` | `{ restored: number, failed: CleanFailure[] }` |
| `apps` | – | `AppInfo[]` |
| `app_storage` | `{ path: string }` (app bundle path, for apps with `storage: true`) | `AppStorage \| null`; its items become cleanable by id through `clean` |
| `uninstall` | `{ paths: string[] }` (app bundle paths) | `CleanResult` (app + its data + leftovers to Trash) |
| `sessions` | – | `AgentTool[]` |
| `memory` | – | `Memory` |
| `memory_update` | `{ op: MemoryOp }` | `Memory` |
| `routine_run` | `{ id: string }` | `CleanResult` |
| `reveal` / `open` / `quicklook` | `{ path }` | `null` |
| `open_full_disk_access` | – | `null` (opens System Settings pane) |

## Events (app → frontend)

| Event | Payload |
|---|---|
| `scan:progress` | `ScanProgress` (~4/s) |
| `scan:tree` | `TreeNode` root, depth 3, while walking (~1/s) and once at the end |
| `clutter:upsert` | `Finding[]` (new or changed; replace by id) |
| `clutter:remove` | `string[]` finding ids that no longer exist |
| `tree:changed` | `{ paths: string[] }` folders whose totals changed (removal or FSEvents); refetch visible |
| `volume` | `VolumeInfo` |
| `memory` | `Memory` (changed, e.g. by an agent over MCP) |

## Apps & sessions types (owned by apps agent, mirrored in `src/lib/types.ts`)

```ts
AppInfo   { name, bundle_id, path, version?, icon: "app:<path>", app_bytes, data_bytes,
            data_paths: string[], last_used?: number, running: boolean, apple: boolean,
            storage: boolean }   // storage: has an `app_storage` page
AppStorage   { app: { name, path, icon, version?, profile: "xcode"|"adobe"|"browser"|"electron" },
               bytes, groups: StorageGroup[] }
StorageGroup { id, title, icon, bytes, note?: string, items: Finding[] }
// The full per-app inventory the user picks from (every simulator, runtime, device-support
// version, DerivedData project, archive, cache folder; Adobe media/disk caches and data of older
// versions; browser caches per profile; Electron caches; older bundled CLI versions). Items are
// ordinary Findings: same keep list, guard, busy checks and actions. Simulators and runtimes are
// removed with `xcrun simctl delete` / `simctl runtime delete` (Command; `env DEVELOPER_DIR=…`
// when xcode-select points at the Command Line Tools); runtime items share id, rule and command
// with the clutter crate's runtime findings. Right before removing, `storage::recheck` keeps
// items that became busy (a simulator booted, an Adobe app opened, a CLI version now in use).
AgentTool { tool: "claude-code"|"codex"|"t3code"|"cursor"|…, name, icon, bytes,
            projects: AgentProject[] }
AgentProject { name, path?: string, bytes, last_used?: number, sessions: AgentSession[] }
AgentSession { id, title?: string, bytes, started?: number, last_used?: number, paths: string[] }
```

## Memory types (owned by memory agent)

```ts
Memory   { keep: KeepEntry[], rules: LearnedRule[], routines: Routine[], community: { enabled, rules: number, updated?: number } }
KeepEntry   { id, pattern, note?, source: "user"|"agent", added: number }      // pattern: absolute path or glob with ~
LearnedRule { id, pattern, category, title, why, regrows, safety, source: "user"|"agent"|"community", added, shared: boolean }
Routine     { id, title, select: { categories?: Category[], rules?: string[] }, every_days, min_age_days, enabled, last_run?: number, last_freed?: number }
MemoryOp    { type: "keep", pattern, note? } | { type: "unkeep", id, source?: "user"|"agent" } | { type: "add-rule", rule }
            | { type: "remove-rule", id } | { type: "set-routine", routine } | { type: "remove-routine", id }
            | { type: "community", enabled }
```
