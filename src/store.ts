import { create } from 'zustand';
import { api, on } from './lib/api';
import { bytes as fmt } from './lib/format';
import type {
  AgentTool, AppInfo, BootInfo, Category, CleanResult, Finding, McpClient, Memory, Safety, ScanProgress, Trashed, TreeNode, VolumeInfo,
} from './lib/types';

export type Section = 'space' | 'clutter' | 'apps' | 'agents' | 'memory' | 'settings';
export type SpaceView = 'sunburst' | 'bubbles' | 'blocks' | 'list';
export type Theme = 'system' | 'dark' | 'light';

/** Anything the user can select anywhere in the app. */
export interface SelItem {
  key: string;
  kind: 'finding' | 'path' | 'app';
  id?: string;
  paths: string[];
  bytes: number;
  title: string;
  icon?: string;
  category?: Category;
  safety: Safety;
  /** It goes to the Trash (Undo); otherwise it's deleted or removed by its tool (simctl…). */
  trash?: boolean;
  /** Removing it can't be undone (emptying the Trash). Needs press-and-hold. */
  permanent?: boolean;
  busy?: string;
  /** Never removable (system and top-level personal folders): only look at it. */
  locked?: string;
}

/** What a right-click opened: the item(s) it acts on, plus the folder it can open in the map. */
export interface MenuState {
  x: number;
  y: number;
  items: SelItem[];
  /** A folder the map can open. */
  folder?: string;
  /** Something Finder can show and Quick Look can preview. */
  path?: string;
}

export interface Toast {
  id: number;
  kind: 'freed' | 'trash' | 'info' | 'error';
  title: string;
  sub?: string;
  undo?: Trashed[];
}

interface State {
  boot: BootInfo | null;
  volume: VolumeInfo | null;
  progress: ScanProgress | null;
  findings: Record<string, Finding>;
  apps: AppInfo[] | null;
  sessions: AgentTool[] | null;
  memory: Memory | null;
  mcp: McpClient[] | null;
  /** Paths removed in this session that the tree may still show: path → bytes. Applied to every tree. */
  gone: Map<string, number>;
  /** Bumped when the disk tree changed (removals, undo, FSEvents): refetch now. */
  treeVersion: number;
  /** Bumped on every streaming scan:tree: refetch at most ~1/s. */
  scanTick: number;
  /** The folder the Space map shows, and its tree as last fetched. */
  spacePath: string;
  spaceTree: TreeNode | null;
  spaceView: SpaceView;
  hover: string | null;
  section: Section;
  selection: Map<string, SelItem>;
  toasts: Toast[];
  theme: Theme;
  dark: boolean;
  busyRemoving: boolean;
  /** Items being dragged right now (the drop dock shows while set). */
  dragging: SelItem[] | null;
  menu: MenuState | null;
  /** Last tree per folder, so coming back to Space (or to a folder) shows it instantly. */
  treeCache: Map<string, TreeNode>;

  init: () => Promise<void>;
  go: (s: Section) => void;
  openFolder: (path: string) => void;
  setView: (v: SpaceView) => void;
  setHover: (p: string | null) => void;
  setSpaceTree: (t: TreeNode | null) => void;
  toggle: (item: SelItem, on?: boolean) => void;
  setMany: (items: SelItem[], on: boolean) => void;
  clearSelection: () => void;
  remove: (items: SelItem[]) => Promise<void>;
  undo: (toastId: number, items: Trashed[]) => Promise<void>;
  toast: (t: Omit<Toast, 'id'>) => void;
  dismiss: (id: number) => void;
  setTheme: (t: Theme) => void;
  refreshApps: () => Promise<void>;
  refreshSessions: () => Promise<void>;
  refreshMemory: () => Promise<void>;
  refreshClutter: () => Promise<void>;
  refreshBoot: () => Promise<void>;
  setDragging: (items: SelItem[] | null) => void;
  openMenu: (m: MenuState) => void;
  closeMenu: () => void;
  keep: (items: SelItem[]) => Promise<void>;
}

const readTheme = (): Theme => {
  try { return (localStorage.getItem('disko.theme') as Theme) || 'system'; } catch { return 'system'; }
};
const readView = (): SpaceView => {
  try { return (localStorage.getItem('disko.view') as SpaceView) || 'sunburst'; } catch { return 'sunburst'; }
};
const mq = typeof window !== 'undefined' ? window.matchMedia('(prefers-color-scheme: dark)') : null;
const resolveDark = (t: Theme) => (t === 'system' ? mq?.matches ?? true : t === 'dark');
function applyTheme(dark: boolean) {
  const el = document.documentElement;
  el.dataset.theme = dark ? 'dark' : 'light';
  el.classList.toggle('dark', dark);
}

let toastId = 0;
/** Finding ids being removed right now; upserts for them are ignored until the clean returns. */
const pending = new Set<string>();
let refreshTimer = 0;
let staleApps = false;
let staleSessions = false;
/** Where coding agents keep their sessions (relative to home). */
const AGENT_DIRS = ['.claude', '.codex', '.t3', '.cursor', 'Library/Application Support/T3 Code'];

export const useStore = create<State>((set, get) => ({
  boot: null,
  volume: null,
  progress: null,
  findings: {},
  apps: null,
  sessions: null,
  memory: null,
  mcp: null,
  gone: new Map(),
  treeVersion: 0,
  scanTick: 0,
  spacePath: '/',
  spaceTree: null,
  spaceView: readView(),
  hover: null,
  section: 'space',
  selection: new Map(),
  toasts: [],
  theme: readTheme(),
  dark: resolveDark(readTheme()),
  busyRemoving: false,
  dragging: null,
  menu: null,
  treeCache: new Map(),

  init: async () => {
    applyTheme(get().dark);
    mq?.addEventListener('change', () => {
      if (get().theme !== 'system') return;
      const dark = resolveDark('system');
      applyTheme(dark);
      set({ dark });
    });
    await Promise.all([
      on('scan:progress', (p) => set({ progress: p })),
      on('scan:tree', () => set((s) => ({ scanTick: s.scanTick + 1 }))),
      on('clutter:upsert', (fs) => set((s) => {
        const next = { ...s.findings };
        for (const f of fs) if (!pending.has(f.id)) next[f.id] = f;
        return { findings: next };
      })),
      on('clutter:remove', (ids) => set((s) => {
        const next = { ...s.findings };
        for (const id of ids) delete next[id];
        const sel = new Map(s.selection);
        for (const id of ids) sel.delete(`f:${id}`);
        return { findings: next, selection: sel };
      })),
      on('tree:changed', ({ paths }) => {
        set((s) => ({ treeVersion: s.treeVersion + 1 }));
        // Apps and sessions are expensive to measure: refresh only when their folders changed,
        // and only while they're on screen (otherwise on the next visit).
        const home = get().boot?.home ?? '';
        const touches = (roots: string[]) => paths.some((p) => roots.some((r) => p === r || p.startsWith(r + '/') || r.startsWith(p + '/')));
        if (get().apps && touches(['/Applications', `${home}/Applications`])) staleApps = true;
        if (get().sessions && touches(AGENT_DIRS.map((d) => `${home}/${d}`))) staleSessions = true;
        if (!staleApps && !staleSessions) return;
        clearTimeout(refreshTimer);
        refreshTimer = window.setTimeout(() => {
          const sec = get().section;
          if (staleApps && sec === 'apps') { staleApps = false; get().refreshApps(); }
          if (staleSessions && sec === 'agents') { staleSessions = false; get().refreshSessions(); }
        }, 1500);
      }),
      on('volume', (v) => set({ volume: v })),
      on('memory', (m) => set({ memory: m })),
    ]);
    const boot = await api.boot();
    set({ boot, volume: boot.volume });
    get().refreshClutter();
    get().refreshMemory();
    // Measure apps and agent sessions in the background so their pages open instantly.
    window.setTimeout(() => { if (!get().apps) get().refreshApps(); }, 4000);
    window.setTimeout(() => { if (!get().sessions) get().refreshSessions(); }, 9000);
  },

  go: (section) => {
    set({ section });
    if (section === 'apps' && (!get().apps || staleApps)) { staleApps = false; get().refreshApps(); }
    if (section === 'agents' && (!get().sessions || staleSessions)) { staleSessions = false; get().refreshSessions(); }
    if (section === 'memory') {
      get().refreshMemory();
      api.mcpStatus().then((mcp) => set({ mcp })).catch(() => {});
    }
  },
  openFolder: (path) => set({ spacePath: path, hover: null, section: 'space' }),
  setView: (spaceView) => {
    try { localStorage.setItem('disko.view', spaceView); } catch { /* private mode */ }
    set({ spaceView });
  },
  setHover: (hover) => { if (get().hover !== hover) set({ hover }); },
  setSpaceTree: (spaceTree) => set({ spaceTree }),

  toggle: (item, onOff) => set((s) => {
    const sel = new Map(s.selection);
    const want = onOff ?? !sel.has(item.key);
    if (want) sel.set(item.key, item); else sel.delete(item.key);
    return { selection: sel };
  }),
  setMany: (items, onOff) => set((s) => {
    const sel = new Map(s.selection);
    for (const it of items) { if (onOff) sel.set(it.key, it); else sel.delete(it.key); }
    return { selection: sel };
  }),
  clearSelection: () => set({ selection: new Map() }),

  remove: async (items) => {
    const busy = items.filter((i) => i.busy || i.locked);
    const todo = items.filter((i) => !i.busy && !i.locked);
    if (busy.length) get().toast({ kind: 'info', title: `Skipped ${busy.length === 1 ? busy[0].title : `${busy.length} items`}`, sub: busy[0].locked ?? busy[0].busy });
    if (!todo.length) return;

    // 1. Make it vanish now.
    const s = get();
    const gone = new Map(s.gone);
    const findings = { ...s.findings };
    const sel = new Map(s.selection);
    let deleted = 0;
    for (const it of todo) {
      sel.delete(it.key);
      for (const p of it.paths) gone.set(p, lookupBytes(s.spaceTree, p) ?? it.bytes / it.paths.length);
      if (it.id) { delete findings[it.id]; pending.add(it.id); }
      if (!it.trash || it.permanent) deleted += it.bytes;
    }
    // Findings fully covered by removed paths go too (e.g. removing a folder from the map).
    for (const f of Object.values(findings)) {
      if (f.paths.length && f.paths.every((p) => [...gone.keys()].some((g) => p === g || p.startsWith(g + '/')))) delete findings[f.id];
    }
    const appPaths = todo.filter((i) => i.kind === 'app').map((i) => i.paths[0]);
    const apps = s.apps?.filter((a) => !appPaths.includes(a.path)) ?? null;
    const sessions = s.sessions ? pruneSessions(s.sessions, gone) : null;
    const volume = s.volume && deleted ? { ...s.volume, used: s.volume.used - deleted, free: s.volume.free + deleted } : s.volume;
    set({ gone, findings, selection: sel, apps, sessions, volume, busyRemoving: true });

    // 2. Do it.
    const ids = todo.filter((i) => i.kind === 'finding').map((i) => i.id!);
    const paths = todo.filter((i) => i.kind === 'path').flatMap((i) => i.paths);
    const results: CleanResult[] = [];
    try {
      if (ids.length || paths.length) results.push(await api.clean(ids, paths));
      if (appPaths.length) results.push(await api.uninstall(appPaths));
    } catch (e) {
      get().toast({ kind: 'error', title: 'Couldn’t remove that', sub: String(e) });
    }
    ids.forEach((id) => pending.delete(id));
    const r = merge(results);

    // 3. Bring back what didn't go, then report.
    const notGone = [...r.failed, ...r.skipped].map((f) => f.path);
    if (notGone.length || !results.length) {
      const g = new Map(get().gone);
      for (const it of todo) for (const p of it.paths) if (!results.length || notGone.includes(p)) g.delete(p);
      set({ gone: g });
      get().refreshClutter();
      if (appPaths.length) get().refreshApps();
    }
    set((st) => ({ busyRemoving: false, treeVersion: st.treeVersion + 1 }));
    const trashedBytes = r.trashed.reduce((a, t) => a + t.bytes, 0);
    if (r.trashed.length) {
      get().toast({ kind: 'trash', title: `${fmt(trashedBytes)} moved to Trash`, sub: r.freed ? `${fmt(r.freed)} freed` : undefined, undo: r.trashed });
    } else if (r.freed) {
      get().toast({ kind: 'freed', title: `${fmt(r.freed)} freed` });
    }
    if (notGone.length) {
      const why = [...r.skipped, ...r.failed][0]?.error;
      get().toast({ kind: 'info', title: `${notGone.length} ${notGone.length === 1 ? 'item' : 'items'} stayed`, sub: why });
    }
    // The tree refetch replaces optimistic state; forget it a bit later.
    window.setTimeout(() => {
      const g = new Map(get().gone);
      for (const it of todo) for (const p of it.paths) g.delete(p);
      set((st) => ({ gone: g, treeVersion: st.treeVersion + 1 }));
    }, 2500);
  },

  undo: async (id, items) => {
    get().dismiss(id);
    // Show it again right away; the refetch confirms.
    const g = new Map(get().gone);
    for (const t of items) g.delete(t.from);
    set({ gone: g });
    try {
      const r = await api.undo(items);
      get().toast({ kind: 'info', title: r.failed.length ? `Put back ${r.restored}, ${r.failed.length} couldn’t` : 'Put back' });
    } catch (e) {
      get().toast({ kind: 'error', title: 'Couldn’t put it back', sub: String(e) });
    }
    get().refreshClutter();
    if (get().apps) get().refreshApps();
    set((s) => ({ treeVersion: s.treeVersion + 1 }));
  },

  toast: (t) => {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts.slice(-2), { ...t, id }] }));
    window.setTimeout(() => get().dismiss(id), t.undo ? 9000 : 4500);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setTheme: (theme) => {
    try { localStorage.setItem('disko.theme', theme); } catch { /* private mode */ }
    const dark = resolveDark(theme);
    applyTheme(dark);
    set({ theme, dark });
  },

  refreshApps: async () => { try { set({ apps: await api.apps() }); } catch { /* keep last */ } },
  refreshSessions: async () => { try { set({ sessions: pruneSessions(await api.sessions(), get().gone) }); } catch { /* keep last */ } },
  refreshMemory: async () => { try { set({ memory: await api.memory() }); } catch { /* keep last */ } },
  refreshClutter: async () => {
    try {
      const list = await api.clutter();
      const findings: Record<string, Finding> = {};
      for (const f of list) if (!pending.has(f.id)) findings[f.id] = f;
      set({ findings });
    } catch { /* keep last */ }
  },
  refreshBoot: async () => { try { const boot = await api.boot(); set({ boot, volume: boot.volume }); } catch { /* keep */ } },
  setDragging: (dragging) => set({ dragging }),
  openMenu: (menu) => set({ menu }),
  closeMenu: () => { if (get().menu) set({ menu: null }); },

  // "Always keep": Disko never suggests or removes these again. Findings they cover vanish now.
  keep: async (items) => {
    const paths = [...new Set(items.flatMap((i) => i.paths))];
    if (!paths.length) return;
    try {
      let memory: Memory | null = null;
      for (const p of paths) memory = await api.memoryUpdate({ type: 'keep', pattern: p });
      const s = get();
      const covered = (fp: string) => paths.some((k) => fp === k || fp.startsWith(k + '/'));
      const findings = { ...s.findings };
      for (const f of Object.values(findings)) if (f.paths.length && f.paths.every(covered)) delete findings[f.id];
      const sel = new Map(s.selection);
      for (const it of items) sel.delete(it.key);
      set({ memory, findings, selection: sel });
      get().toast({ kind: 'info', title: items.length === 1 ? `Keeping ${items[0].title}` : `Keeping ${items.length} items`, sub: 'Disko won’t suggest it again. Undo in Memory.' });
      get().refreshClutter();
    } catch (e) {
      get().toast({ kind: 'error', title: 'Couldn’t keep that', sub: String(e) });
    }
  },
}));

function merge(rs: CleanResult[]): CleanResult {
  return rs.reduce((a, r) => ({
    freed: a.freed + r.freed, removed: [...a.removed, ...r.removed], trashed: [...a.trashed, ...r.trashed],
    failed: [...a.failed, ...r.failed], skipped: [...a.skipped, ...r.skipped],
  }), { freed: 0, removed: [], trashed: [], failed: [], skipped: [] } as CleanResult);
}

function lookupBytes(tree: TreeNode | null, path: string): number | null {
  let n: TreeNode | undefined | null = tree;
  while (n) {
    if (n.path === path) return n.bytes;
    n = n.children?.find((c) => path === c.path || path.startsWith(c.path + '/'));
  }
  return null;
}

const isGone = (gone: Map<string, number>, p: string) => {
  for (const g of gone.keys()) if (p === g || p.startsWith(g + '/')) return true;
  return false;
};

/** Removes optimistically deleted paths from a tree, shrinking every ancestor. */
export function applyGone(tree: TreeNode, gone: Map<string, number>): TreeNode {
  if (!gone.size) return tree;
  const relevant = [...gone].filter(([p]) => p.startsWith(tree.path === '/' ? '/' : tree.path + '/'));
  if (!relevant.length) return tree;
  const walk = (n: TreeNode): TreeNode => {
    let minus = 0;
    for (const [p, b] of relevant) if (p.startsWith(n.path === '/' ? '/' : n.path + '/')) minus += b;
    if (!minus) return n;
    const children = n.children
      ?.filter((c) => !gone.has(c.path))
      .map(walk);
    return { ...n, bytes: Math.max(0, n.bytes - minus), children };
  };
  return walk(tree);
}

function pruneSessions(tools: AgentTool[], gone: Map<string, number>): AgentTool[] {
  if (!gone.size) return tools;
  return tools.map((t) => {
    const projects = t.projects.map((p) => {
      const sessions = p.sessions.filter((s) => !s.paths.every((x) => isGone(gone, x)));
      const bytes = p.bytes - p.sessions.filter((s) => !sessions.includes(s)).reduce((a, s) => a + s.bytes, 0);
      return { ...p, sessions, bytes };
    }).filter((p) => p.sessions.length > 0);
    return { ...t, projects, bytes: projects.reduce((a, p) => a + p.bytes, 0) };
  });
}

/** Selection helpers so every section builds items the same way. */
export const sel = {
  finding: (f: Finding): SelItem => ({
    key: `f:${f.id}`, kind: 'finding', id: f.id, paths: f.paths, bytes: f.bytes, title: f.title, icon: f.icon,
    category: f.category, safety: f.safety, permanent: f.category === 'trash', busy: f.busy,
    trash: f.action.type === 'trash' || (f.safety === 'review' && f.action.type === 'delete'),
  }),
  node: (n: TreeNode): SelItem => ({
    key: `p:${n.path}`, kind: 'path', paths: [n.path], bytes: n.bytes, title: n.name, category: n.clutter,
    safety: 'review', trash: true, permanent: n.clutter === 'trash' && n.path.includes('/.Trash/'),
    locked: n.protected ? 'Protected' : undefined,
  }),
  app: (a: AppInfo): SelItem => ({
    key: `a:${a.path}`, kind: 'app', paths: [a.path, ...a.data_paths], bytes: a.app_bytes + a.data_bytes, title: a.name,
    icon: a.icon, safety: 'review', trash: true, busy: a.running ? `${a.name} is open` : undefined,
  }),
  paths: (key: string, title: string, paths: string[], bytes: number, icon?: string): SelItem => ({
    key, kind: 'path', paths, bytes, title, icon, category: 'agent-data', safety: 'review', trash: true,
  }),
};
