# Disko 1.0 frontend

- `main.tsx` mounts the app and makes every `.drag-region` move the window (WKWebView ignores `-webkit-app-region`).
- `App.tsx` is the shell: icon `Rail`, draggable `TopBar` (disk capsule + scan ring), the current section, the floating `SelectionBar`, `Toasts` (Undo) and `UpdatePrompt`.
- `store.ts` (zustand) holds everything plus the one removal flow: anything selected anywhere becomes a `SelItem`, `remove()` hides it right away (`gone`), calls `clean`/`uninstall`, and offers Undo for trashed items.
- `lib/api.ts` is the only door to the app (Tauri `invoke`/`listen`). Without `window.__TAURI_INTERNALS__` it uses `lib/mock.ts`, a fake Mac with a streaming ~8 s scan, so `bun run dev` shows the whole UI.
- `lib/types.ts` mirrors `crates/core/src/types.rs` and docs/contract.md (snake_case). `lib/categories.ts` defines each category's colour + icon once. `lib/icons.tsx` has glyphs, app icons and node icons.
- `sections/space/`: `model.ts` (fetch + live refresh + hidden-space slices), `Sunburst` (canvas, clutter halo), `Bubbles` (pack), `Blocks` (treemap), `FolderList` (side list + list view).
- `sections/`: `Clutter`, `Apps` (with `AppStorage`, an app's storage page: every simulator, runtime, cache …), `Agents`, `Memory`, `Settings`.
