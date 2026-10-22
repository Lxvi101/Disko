import { useCallback, useMemo } from 'react';
import { create } from 'zustand';
import type {
  AppInfo,
  CandidateItem,
  ChatMessage,
  CodexModels,
  FileItem,
  OverviewData,
  Page,
  QuarantineJournal,
  RemnantsReport,
  ScanInfo,
  ScanProgress,
  StagedItem,
  UnusedItem,
  VizMode,
} from './types';
import { api, errorText } from './lib/api';
import { uid } from './lib/format';

/** An item taken out of the disk map so the rest can use its space. */
export interface HiddenItem {
  path: string;
  name: string;
  kind: string;
  total: number;
  /** Remembered across launches; otherwise it reappears next time Disko starts. */
  persist: boolean;
}

const HIDDEN_KEY = 'disko.hidden';
const saveHidden = (items: HiddenItem[]) => {
  try { localStorage.setItem(HIDDEN_KEY, JSON.stringify(items.filter((i) => i.persist))); } catch {}
};

export interface Toast {
  id: string;
  kind: 'info' | 'success' | 'error';
  title: string;
  detail?: string;
}

interface AppState {
  // boot
  booted: boolean;
  bootError: string | null;
  info: AppInfo | null;
  overview: OverviewData | null;
  models: CodexModels | null;

  // navigation
  page: Page;
  viz: VizMode;
  currentPath: string;
  rootPath: string;
  currentItems: FileItem[];
  itemsLoading: boolean;
  selectedPath: string | null;
  history: string[];
  historyIdx: number;
  hoverPath: string | null;
  advanced: boolean;
  colorBy: 'folder' | 'policy';
  theme: 'system' | 'light' | 'dark';
  resolvedTheme: 'light' | 'dark';

  /** Items hidden from the disk map. */
  hiddenItems: HiddenItem[];

  // data
  candidates: CandidateItem[];
  unusedFiles: UnusedItem[];
  unusedApps: UnusedItem[];
  remnants: RemnantsReport | null;
  largestFiles: FileItem[];
  journals: QuarantineJournal[];
  coverageIssues: number;
  /** Bumped whenever the loaded scan changes underneath the views (quarantine move or restore). */
  dataVersion: number;

  // cleanup
  staged: StagedItem[];
  trayOpen: boolean;
  cleanModalOpen: boolean;
  cleanupMode: 'quarantine' | 'delete';
  cleanupBusy: boolean;
  cleanupSelection: StagedItem[] | null;

  // assistant
  assistantOpen: boolean;
  messages: ChatMessage[];
  generating: string | null; // request id
  model: string;
  effort: string;

  // misc
  startScreen: boolean;
  scans: ScanInfo[];
  autoscan: string | null;
  paletteOpen: boolean;
  scan: ScanProgress | null;
  toasts: Toast[];

  // actions
  boot: () => Promise<void>;
  reloadData: () => Promise<void>;
  setPage: (p: Page) => void;
  setViz: (v: VizMode) => void;
  hideItem: (it: Omit<HiddenItem, 'persist'>) => void;
  unhideItems: (paths: string[]) => void;
  setHiddenPersist: (path: string, persist: boolean) => void;
  navigate: (path: string, opts?: { fromHistory?: boolean }) => Promise<void>;
  navigateUp: () => Promise<void>;
  select: (path: string | null) => void;
  setHover: (p: string | null) => void;
  setAdvanced: (v: boolean) => void;
  setColorBy: (v: 'folder' | 'policy') => void;
  setTheme: (t: 'system' | 'light' | 'dark') => void;
  applyTheme: () => void;
  goBack: () => void;
  goForward: () => void;
  canBack: () => boolean;
  canForward: () => boolean;
  toggleStage: (item: Omit<StagedItem, 'source'> & { source?: StagedItem['source'] }) => void;
  stageMany: (items: StagedItem[]) => void;
  unstage: (path: string) => void;
  clearStaged: () => void;
  isStaged: (path: string) => boolean;
  setTrayOpen: (v: boolean) => void;
  setCleanModalOpen: (v: boolean) => void;
  setAssistantOpen: (v: boolean) => void;
  setModel: (m: string) => void;
  setEffort: (e: string) => void;
  pushMessage: (m: ChatMessage) => void;
  updateMessage: (id: string, patch: Partial<ChatMessage> | ((m: ChatMessage) => Partial<ChatMessage>)) => void;
  setGenerating: (id: string | null) => void;
  clearChat: () => void;
  setPaletteOpen: (v: boolean) => void;
  setStartScreen: (v: boolean) => void;
  refreshScans: () => Promise<void>;
  setScan: (s: ScanProgress | null) => void;
  toast: (t: Omit<Toast, 'id'>) => void;
  dismissToast: (id: string) => void;
  refreshJournals: () => Promise<void>;
  refreshCurrent: () => Promise<void>;
}

let launchApplied = false;

export const useStore = create<AppState>((set, get) => ({
  booted: false,
  bootError: null,
  info: null,
  overview: null,
  models: null,

  page: 'quick',
  viz: (() => { try { const v = localStorage.getItem('disko.viz'); return v === 'treemap' || v === 'timeline' ? v : 'sunburst'; } catch { return 'sunburst'; } })(),
  hiddenItems: (() => {
    try {
      const raw = JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? '[]');
      return Array.isArray(raw) ? raw.filter((i) => typeof i?.path === 'string').map((i) => ({ ...i, persist: true })) : [];
    } catch { return []; }
  })(),
  currentPath: '',
  rootPath: '',
  currentItems: [],
  itemsLoading: false,
  dataVersion: 0,
  selectedPath: null,
  history: [],
  historyIdx: -1,
  hoverPath: null,
  advanced: (() => { try { return (localStorage.getItem('disko.advanced')) === '1'; } catch { return false; } })(),
  colorBy: 'folder',
  theme: (() => { try { return ((localStorage.getItem('disko.theme')) as any) || 'system'; } catch { return 'system'; } })(),
  resolvedTheme: 'dark',

  candidates: [],
  unusedFiles: [],
  unusedApps: [],
  remnants: null,
  largestFiles: [],
  journals: [],
  coverageIssues: 0,

  staged: [],
  trayOpen: false,
  cleanModalOpen: false,
  cleanupMode: 'quarantine',
  cleanupBusy: false,
  cleanupSelection: null,

  assistantOpen: false,
  messages: [],
  generating: null,
  // '' means "whatever Codex picks": the model and effort from ~/.codex/config.toml.
  model: (() => { try { return localStorage.getItem('disko.model') ?? ''; } catch { return ''; } })(),
  effort: (() => { try { return localStorage.getItem('disko.effort') ?? ''; } catch { return ''; } })(),

  startScreen: false,
  scans: [],
  autoscan: null,
  paletteOpen: false,
  scan: null,
  toasts: [],

  boot: async () => {
    get().applyTheme();
    try {
      const [info, overview] = await Promise.all([api.appInfo(), api.overview()]);
      set({
        info,
        overview,
        rootPath: overview.root,
        currentPath: overview.root,
        currentItems: overview.items,
        history: [overview.root],
        historyIdx: 0,
        hoverPath: null,
        selectedPath: null,
        booted: true,
      });
      api
        .models()
        .then((models) =>
          set((s) => ({
            models,
            // A remembered model that this Codex no longer offers falls back to Codex's own default.
            model: s.model && models.models.some((m) => m.slug === s.model) ? s.model : '',
            effort: s.model && models.models.some((m) => m.slug === s.model) ? s.effort : '',
          })),
        )
        .catch(() => {});
      get().reloadData();
      // Optional launch overrides (used for automation and debugging); applied once per process.
      if (launchApplied) return;
      launchApplied = true;
      api
        .launchOptions()
        .then(async (o) => {
          if (o?.advanced != null) get().setAdvanced(o.advanced);
          if (o?.start_path) await get().navigate(o.start_path);
          if (o?.start_page) set({ page: (({ files: 'junk', inactive: 'timeline' } as Record<string, string>)[o.start_page] ?? o.start_page) as Page, startScreen: false });
          if (o?.start_path) set({ startScreen: false });
          if (o?.autoscan) set({ autoscan: o.autoscan, startScreen: true });
        })
        .catch(() => {});
    } catch (e) {
      set({ bootError: errorText(e), booted: true });
    }
  },

  reloadData: async () => {
    const [candidates, unusedFiles, unusedApps, journals, largestFiles] = await Promise.all([
      api.candidates().catch(() => []),
      api.unusedFiles().catch(() => []),
      api.unusedApps().catch(() => []),
      api.journals().catch(() => []),
      api.largestFiles(40).catch(() => []),
    ]);
    set({ candidates, unusedFiles, unusedApps, journals, largestFiles });
    // Leftover detection shells out to Spotlight and may take a few seconds; never block the rest.
    api.remnants().then((remnants) => set({ remnants })).catch(() => {});
    api.coverageIssues().then((coverageIssues) => set({ coverageIssues })).catch(() => {});
  },

  setPage: (page) => set({ page }),
  setViz: (viz) => {
    try { localStorage.setItem('disko.viz', viz); } catch {}
    set({ viz });
  },
  hideItem: (it) => set((s) => {
    if (s.hiddenItems.some((h) => h.path === it.path)) return {};
    // Hiding a folder covers anything already hidden inside it.
    const hiddenItems = [...s.hiddenItems.filter((h) => !h.path.startsWith(it.path + '/')), { ...it, persist: false }];
    saveHidden(hiddenItems);
    return { hiddenItems, hoverPath: null };
  }),
  unhideItems: (paths) => set((s) => {
    const hiddenItems = s.hiddenItems.filter((h) => !paths.includes(h.path));
    saveHidden(hiddenItems);
    return { hiddenItems };
  }),
  setHiddenPersist: (path, persist) => set((s) => {
    const hiddenItems = s.hiddenItems.map((h) => (h.path === path ? { ...h, persist } : h));
    saveHidden(hiddenItems);
    return { hiddenItems };
  }),

  navigate: async (path, opts?: { fromHistory?: boolean }) => {
    const { currentPath } = get();
    set({ itemsLoading: true, page: 'explore', selectedPath: null, hoverPath: null });
    try {
      const items = await api.children(path);
      set((s) => {
        if (opts?.fromHistory || path === currentPath) return { currentItems: items, currentPath: path };
        const hist = [...s.history.slice(0, s.historyIdx + 1), path].slice(-60);
        return { currentItems: items, currentPath: path, history: hist, historyIdx: hist.length - 1 };
      });
    } catch (e) {
      get().toast({ kind: 'error', title: 'Could not open folder', detail: errorText(e) });
    } finally {
      set({ itemsLoading: false });
    }
  },

  navigateUp: async () => {
    const { currentPath, rootPath } = get();
    if (!currentPath || currentPath === rootPath) return;
    const idx = currentPath.lastIndexOf('/');
    const parent = idx > 0 ? currentPath.slice(0, idx) : rootPath;
    await get().navigate(parent.length < rootPath.length ? rootPath : parent);
  },

  select: (selectedPath) => set({ selectedPath }),
  setHover: (hoverPath) => set({ hoverPath }),
  setAdvanced: (advanced) => {
    try { localStorage.setItem('disko.advanced', advanced ? '1' : '0'); } catch {}
    set({ advanced });
  },
  setColorBy: (colorBy) => set({ colorBy }),
  setTheme: (theme) => {
    try { localStorage.setItem('disko.theme', theme); } catch {}
    set({ theme });
    get().applyTheme();
  },
  applyTheme: () => {
    const { theme } = get();
    const sys = window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
    const resolved = theme === 'system' ? sys : theme;
    document.documentElement.dataset.theme = resolved;
    set({ resolvedTheme: resolved });
    // Keep the native window colour in sync so resizing never flashes the other theme.
    import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => {
        const c = resolved === 'light' ? { red: 243, green: 243, blue: 247, alpha: 1 } : { red: 15, green: 16, blue: 21, alpha: 1 };
        return getCurrentWindow().setBackgroundColor(c);
      })
      .catch(() => {});
  },
  canBack: () => get().historyIdx > 0,
  canForward: () => get().historyIdx < get().history.length - 1,
  goBack: () => {
    const { history, historyIdx } = get();
    if (historyIdx <= 0) return;
    set({ historyIdx: historyIdx - 1 });
    get().navigate(history[historyIdx - 1], { fromHistory: true });
  },
  goForward: () => {
    const { history, historyIdx } = get();
    if (historyIdx >= history.length - 1) return;
    set({ historyIdx: historyIdx + 1 });
    get().navigate(history[historyIdx + 1], { fromHistory: true });
  },

  toggleStage: (item) => {
    const { staged } = get();
    if (staged.some((s) => s.path === item.path)) {
      set({ staged: staged.filter((s) => s.path !== item.path) });
      return;
    }
    // Drop descendants/ancestors overlap: keep the new one, remove items inside it.
    const filtered = staged.filter((s) => !s.path.startsWith(item.path + '/'));
    if (staged.some((s) => item.path.startsWith(s.path + '/'))) {
      get().toast({ kind: 'info', title: 'Already covered', detail: 'A parent folder of this item is already staged.' });
      return;
    }
    set({ staged: [...filtered, { ...item, source: item.source ?? 'explore' }] });
  },

  stageMany: (items) => {
    const { staged } = get();
    const next = [...staged];
    for (const it of items) {
      if (next.some((s) => s.path === it.path || it.path.startsWith(s.path + '/'))) continue;
      for (let i = next.length - 1; i >= 0; i--) {
        if (next[i].path.startsWith(it.path + '/')) next.splice(i, 1);
      }
      next.push(it);
    }
    set({ staged: next });
  },

  unstage: (path) => set((s) => ({ staged: s.staged.filter((x) => x.path !== path) })),
  clearStaged: () => set({ staged: [], trayOpen: false }),
  isStaged: (path) => get().staged.some((s) => s.path === path),
  setTrayOpen: (trayOpen) => set({ trayOpen }),
  setCleanModalOpen: (cleanModalOpen) => { if (!get().cleanupBusy) set({ cleanModalOpen }); },

  setAssistantOpen: (assistantOpen) => set({ assistantOpen }),
  setModel: (model) => {
    const m = get().models?.models.find((x) => x.slug === model);
    const effort = m && m.efforts.includes(get().effort) ? get().effort : '';
    try { localStorage.setItem('disko.model', model); localStorage.setItem('disko.effort', effort); } catch {}
    set({ model, effort });
  },
  setEffort: (effort) => { try { localStorage.setItem('disko.effort', effort); } catch {} set({ effort }); },
  pushMessage: (m) => set((s) => ({ messages: [...s.messages, m] })),
  updateMessage: (id, patch) =>
    set((s) => ({
      messages: s.messages.map((m) => (m.id === id ? { ...m, ...(typeof patch === 'function' ? patch(m) : patch) } : m)),
    })),
  setGenerating: (generating) => set({ generating }),
  clearChat: () => set({ messages: [] }),

  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setStartScreen: (startScreen) => set({ startScreen }),
  refreshScans: async () => {
    try {
      set({ scans: await api.listScans() });
    } catch {}
  },
  setScan: (scan) => set({ scan }),

  toast: (t) => {
    const id = uid();
    set((s) => ({ toasts: [...s.toasts.slice(-3), { ...t, id }] }));
    setTimeout(() => get().dismissToast(id), t.kind === 'error' ? 7000 : 4000);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  refreshJournals: async () => {
    try {
      set({ journals: await api.journals() });
    } catch {}
  },
  refreshCurrent: async () => {
    const { currentPath } = get();
    if (!currentPath) return;
    try {
      const [items, overview] = await Promise.all([api.children(currentPath), api.overview()]);
      set((s) => ({
        currentItems: items,
        overview,
        dataVersion: s.dataVersion + 1,
        selectedPath: s.selectedPath && !items.some((i) => i.path === s.selectedPath) ? null : s.selectedPath,
      }));
    } catch {}
  },
}));

/** A path counts as collected when it, or a folder containing it, is in the Collector. */
class CollectedSet extends Set<string> {
  has(path: string): boolean {
    if (super.has(path)) return true;
    for (const p of this) if (path.startsWith(p + '/')) return true;
    return false;
  }
}

/**
 * Checkboxes on every page are the Collector: an item is ticked exactly when it is collected,
 * so there is one selection and one Clean up button for the whole app.
 * `lookup` turns a ticked path into the item to collect.
 */
export function useCollectedSelection(lookup: (path: string) => StagedItem | undefined) {
  const staged = useStore((s) => s.staged);
  const selected = useMemo(() => new CollectedSet(staged.map((s) => s.path)), [staged]);
  const setSelected = useCallback((next: Set<string> | ((old: Set<string>) => Set<string>)) => {
    const s = useStore.getState();
    const base = new Set(s.staged.map((i) => i.path));
    const target = typeof next === 'function' ? next(new CollectedSet(base)) : next;
    for (const p of base) if (!target.has(p)) s.unstage(p);
    const added = [...target].filter((p) => !base.has(p)).map(lookup).filter((i): i is StagedItem => !!i);
    if (added.length) useStore.getState().stageMany(added);
  }, [lookup]);
  return [selected, setSelected] as const;
}
