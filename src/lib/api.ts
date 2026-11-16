// The only place the UI talks to the app. In a plain browser (no Tauri) everything is served by
// `mock.ts`, a fake Mac, so `bun run dev` shows the whole app.
import type {
  AgentTool, AppInfo, AppStorage, BootInfo, CleanFailure, CleanResult, Events, Finding, McpClient, Memory,
  MemoryOp, Trashed, TreeNode,
} from './types';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Listen = <K extends keyof Events>(event: K, cb: (payload: Events[K]) => void) => Promise<() => void>;

let backend: Promise<{ invoke: Invoke; listen: Listen }> | null = null;

function load() {
  if (backend) return backend;
  backend = isTauri
    ? Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(([core, ev]) => ({
        invoke: core.invoke as Invoke,
        listen: (async (event, cb) => ev.listen(event, (e) => cb(e.payload as never))) as Listen,
      }))
    : import('./mock').then((m) => ({ invoke: m.mockInvoke as Invoke, listen: m.mockListen as Listen }));
  return backend;
}

const call = async <T>(cmd: string, args?: Record<string, unknown>) => (await load()).invoke<T>(cmd, args);

export const api = {
  boot: () => call<BootInfo>('boot'),
  rescan: () => call<null>('rescan'),
  tree: (path: string | null, depth: number) => call<TreeNode>('tree', { path, depth }),
  search: (query: string, limit = 50) => call<TreeNode[]>('search', { query, limit }),
  clutter: () => call<Finding[]>('clutter'),
  clean: (ids: string[], paths: string[]) => call<CleanResult>('clean', { ids, paths }),
  undo: (trashed: Trashed[]) => call<{ restored: number; failed: CleanFailure[] }>('undo', { trashed }),
  apps: () => call<AppInfo[]>('apps'),
  appStorage: (path: string) => call<AppStorage | null>('app_storage', { path }),
  uninstall: (paths: string[]) => call<CleanResult>('uninstall', { paths }),
  sessions: () => call<AgentTool[]>('sessions'),
  memory: () => call<Memory>('memory'),
  memoryUpdate: (op: MemoryOp) => call<Memory>('memory_update', { op }),
  routineRun: (id: string) => call<CleanResult>('routine_run', { id }),
  reveal: (path: string) => call<null>('reveal', { path }),
  open: (path: string) => call<null>('open', { path }),
  quicklook: (path: string) => call<null>('quicklook', { path }),
  openFullDiskAccess: () => call<null>('open_full_disk_access'),
  // Not yet in docs/contract.md (requested by the frontend):
  icon: (key: string, size: number) => call<string | null>('icon', { key, size }),
  mcpStatus: () => call<McpClient[]>('mcp_status'),
  mcpInstall: (client: string) => call<McpClient[]>('mcp_install', { client }),
};

export async function on<K extends keyof Events>(event: K, cb: (payload: Events[K]) => void) {
  return (await load()).listen(event, cb);
}
