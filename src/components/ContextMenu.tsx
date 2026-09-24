import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { create } from 'zustand';
import { motion, AnimatePresence } from 'framer-motion';
import { useStore } from '../store';
import { api } from '../lib/api';
import { formatBytes, shortPath } from '../lib/format';
import type { StagedItem } from '../types';

export interface MenuItem {
  label: string;
  hint?: string;
  shortcut?: 'preview' | 'collect';
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  onSelect?: () => void;
}
export type MenuEntry = MenuItem | 'sep';

interface MenuState {
  open: boolean;
  x: number;
  y: number;
  header: string | null;
  sub: string | null;
  entries: MenuEntry[];
  show: (x: number, y: number, entries: MenuEntry[], header?: string | null, sub?: string | null) => void;
  hide: () => void;
}

const useMenu = create<MenuState>((set) => ({
  open: false,
  x: 0,
  y: 0,
  header: null,
  sub: null,
  entries: [],
  show: (x, y, entries, header = null, sub = null) => set({ open: true, x, y, entries, header, sub }),
  hide: () => set({ open: false }),
}));

export const openMenu = (e: React.MouseEvent | MouseEvent, entries: MenuEntry[], header?: string | null, sub?: string | null) => {
  e.preventDefault();
  e.stopPropagation();
  useMenu.getState().show(e.clientX, e.clientY, entries, header, sub);
};

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.cssText = 'position:fixed;top:-1000px;left:-1000px;opacity:0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

/** The path-shaped subset every list in the app can hand over. */
export type MenuTarget = Pick<StagedItem, 'path' | 'name' | 'kind' | 'total' | 'category' | 'reason'> & Partial<Pick<StagedItem, 'tag' | 'action' | 'tool'>> & { source?: StagedItem['source'] };

function cleanupEntries(it: MenuTarget): MenuEntry[] {
  return (['quarantine', 'delete'] as const).map((mode) => ({
    label: mode === 'delete' ? 'Delete permanently…' : 'Quarantine…',
    danger: mode === 'delete',
    disabled: it.category === 'protected' || useStore.getState().cleanupBusy,
    onSelect: () => {
      if (useStore.getState().cleanupBusy) return;
      useStore.setState({ cleanupSelection: [{ ...it, source: it.source ?? 'explore' }], cleanupMode: mode, cleanModalOpen: true });
    },
  }));
}

/** Stage an item from a pointer event and fly a marker from the cursor to the collector button. */
export const collectFromPointer = (e: { clientX: number; clientY: number }, it: MenuTarget, color?: string) => {
  const s = useStore.getState();
  const name = it.name || it.path.split('/').pop() || it.path;
  if (s.isStaged(it.path)) {
    s.unstage(it.path);
    return;
  }
  if (it.category === 'protected') {
    s.toast({ kind: 'info', title: `${name} cannot be collected`, detail: it.reason || 'Protected: system, credential or cloud-managed path.' });
    return;
  }
  const before = s.staged.length;
  s.toggleStage({ ...it, name, source: it.source ?? 'explore' });
  if (useStore.getState().staged.length <= before) return; // refused (already covered by a parent)
  flyToCollector(e.clientX, e.clientY, color);
};

export function flyToCollector(x: number, y: number, color?: string) {
  const target = document.getElementById('disko-collector-target');
  if (!target) return;
  const r = target.getBoundingClientRect();
  const dot = document.createElement('div');
  dot.style.cssText = `position:fixed;left:${x - 7}px;top:${y - 7}px;width:14px;height:14px;border-radius:9999px;pointer-events:none;z-index:120;background:${color ?? 'var(--text)'};box-shadow:0 0 0 2px var(--bg)`;
  document.body.appendChild(dot);
  const anim = dot.animate(
    [
      { transform: 'translate(0,0) scale(1)', opacity: 1 },
      { transform: `translate(${r.left + r.width / 2 - x}px, ${r.top + r.height / 2 - y}px) scale(0.5)`, opacity: 0.9 },
    ],
    { duration: 420, easing: 'cubic-bezier(0.2, 0.7, 0.2, 1)' },
  );
  anim.onfinish = () => {
    dot.remove();
    target.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.18)' }, { transform: 'scale(1)' }], { duration: 260, easing: 'ease-out' });
  };
}

/** Compact Finder-style menu shared by the disk map and its folder list. */
export const openExploreMenu = (e: React.MouseEvent | MouseEvent, it: MenuTarget, color?: string) => {
  const s = useStore.getState();
  const name = it.name || it.path.split('/').pop() || it.path;
  const staged = s.isStaged(it.path);
  const origin = { clientX: e.clientX, clientY: e.clientY };
  const run = (action: () => Promise<void>, title: string) => () => {
    void action().catch((err) => s.toast({ kind: 'error', title, detail: String(err) }));
  };
  openMenu(e, [
    it.kind === 'directory'
      ? { label: `Expand “${name}”`, onSelect: () => { void s.navigate(it.path); } }
      : { label: `Open “${name}”`, onSelect: run(() => api.open(it.path), 'Could not open') },
    { label: 'Preview', hint: 'Space', shortcut: 'preview', onSelect: run(() => api.preview(it.path), 'Could not preview') },
    { label: 'Show in Finder', onSelect: run(() => api.reveal(it.path), 'Could not show in Finder') },
    { label: 'Open in Terminal', onSelect: run(() => api.openTerminal(it.path), 'Could not open Terminal') },
    ...cleanupEntries(it),
    {
      label: staged ? `Remove “${name}” from Collector` : `Move “${name}” to Collector`,
      hint: '⌘⌫',
      shortcut: 'collect',
      disabled: it.category === 'protected' && !staged,
      title: it.category === 'protected' ? it.reason || 'Protected: system, credential or cloud-managed path.' : undefined,
      onSelect: () => collectFromPointer(origin, it, color),
    },
  ]);
};

/** Extended right-click menu for secondary pages and collector items. */
export const openItemMenu = (e: React.MouseEvent | MouseEvent, it: MenuTarget) => {
  const s = useStore.getState();
  const isDir = it.kind === 'directory';
  const staged = s.isStaged(it.path);
  const protectedItem = it.category === 'protected';
  const name = it.name || it.path.split('/').pop() || it.path;
  const inScan = s.page === 'explore' || s.page === 'files' ? true : it.path.startsWith(s.rootPath);

  const entries: MenuEntry[] = [
    ...cleanupEntries(it),
    {
      label: staged ? 'Remove from collection' : 'Collect for cleanup',
      hint: staged ? undefined : formatBytes(it.total),
      disabled: protectedItem && !staged,
      title: protectedItem ? it.reason || 'Protected: system, credential or cloud-managed path.' : undefined,
      onSelect: () => (staged ? s.unstage(it.path) : s.toggleStage({ ...it, name, source: it.source ?? 'explore' })),
    },
    'sep',
    ...(isDir && inScan
      ? [{ label: 'Open in Disko', onSelect: () => { s.setPage('explore'); s.navigate(it.path); } } as MenuItem]
      : []),
    { label: isDir ? 'Open in Finder' : 'Open', onSelect: () => api.open(it.path).catch((err) => s.toast({ kind: 'error', title: 'Could not open', detail: String(err) })) },
    { label: 'Reveal in Finder', onSelect: () => api.reveal(it.path).catch(() => {}) },
    'sep',
    ...(s.advanced && (s.page === 'explore' || s.page === 'files') ? [{ label: 'Inspect', onSelect: () => s.select(it.path) } as MenuItem] : []),
    { label: 'Copy path', hint: shortPath(it.path, s.info?.home ?? ''), onSelect: async () => { if (await copyText(it.path)) s.toast({ kind: 'success', title: 'Path copied' }); } },
    { label: 'Copy name', onSelect: async () => { if (await copyText(name)) s.toast({ kind: 'success', title: 'Name copied' }); } },
    'sep',
    {
      label: 'Ask Disko about this',
      onSelect: () => {
        s.setAssistantOpen(true);
        const q = `Tell me about ${it.path} (${formatBytes(it.total)}, ${it.category}${it.reason ? `: ${it.reason}` : ''}). What is it, is it safe to remove, and what would it cost to recreate?`;
        setTimeout(() => window.dispatchEvent(new CustomEvent('disko:ask', { detail: q })), 50);
      },
    },
  ];
  openMenu(e, entries, name, `${formatBytes(it.total)} · ${it.category}`);
};

const PAD = 8;

export const ContextMenuHost: React.FC = () => {
  const { open, x, y, entries, header, sub, hide } = useMenu();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    if (!open) return;
    const el = ref.current;
    const w = el?.offsetWidth ?? 220;
    const h = el?.offsetHeight ?? 200;
    setPos({
      left: Math.max(PAD, Math.min(x, window.innerWidth - w - PAD)),
      top: Math.max(PAD, Math.min(y, window.innerHeight - h - PAD)),
    });
  }, [open, x, y, entries]);

  useEffect(() => {
    if (!open) return;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
    const close = () => hide();
    const outside = (e: MouseEvent) => {
      if (e.target instanceof Node && !ref.current?.contains(e.target)) hide();
    };
    const key = (e: KeyboardEvent) => {
      const shortcut = e.key === ' ' && !e.metaKey && !e.ctrlKey && !e.altKey ? 'preview'
        : (e.metaKey || e.ctrlKey) && e.key === 'Backspace' ? 'collect' : null;
      const action = entries.find((entry): entry is MenuItem => entry !== 'sep' && entry.shortcut === shortcut && !entry.disabled);
      if (shortcut && entries.some((entry) => entry !== 'sep' && entry.shortcut === shortcut)) {
        e.preventDefault();
        e.stopImmediatePropagation();
        if (action) {
          hide();
          action.onSelect?.();
        }
      } else if (e.key === 'Escape' || e.key === 'Tab') {
        e.preventDefault();
        e.stopImmediatePropagation();
        hide();
      } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
        e.preventDefault();
        e.stopImmediatePropagation();
        const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []);
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = e.key === 'Home' ? 0 : e.key === 'End' ? buttons.length - 1
          : (index + (e.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }
    };
    window.addEventListener('mousedown', outside, true);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    window.addEventListener('keydown', key, true);
    window.addEventListener('wheel', close, { capture: true, passive: true });
    return () => {
      window.removeEventListener('mousedown', outside, true);
      window.removeEventListener('blur', close);
      window.removeEventListener('resize', close);
      window.removeEventListener('keydown', key, true);
      window.removeEventListener('wheel', close, true);
      previousFocus?.focus({ preventScroll: true });
    };
  }, [open, entries, hide]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          ref={ref}
          key="ctx"
          role="menu"
          initial={{ opacity: 0, scale: 0.96 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.98 }}
          transition={{ duration: 0.1 }}
          className="fixed z-[95] panel-raised !rounded-lg py-1 min-w-[220px] max-w-[min(520px,calc(100vw-16px))] select-none"
          style={{ left: pos.left, top: pos.top, transformOrigin: 'top left' }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          {header && (
            <div className="px-3 pt-1.5 pb-2 mb-1" style={{ borderBottom: '1px solid var(--line)' }}>
              <div className="text-[12.5px] font-semibold truncate">{header}</div>
              {sub && <div className="text-[10.5px] truncate tnum" style={{ color: 'var(--dim)' }}>{sub}</div>}
            </div>
          )}
          {entries.map((en, i) =>
            en === 'sep' ? (
              <div key={i} className="my-1 h-px mx-2" style={{ background: 'var(--line)' }} />
            ) : (
              <button
                key={i}
                role="menuitem"
                disabled={en.disabled}
                title={en.title}
                className="w-full flex items-center gap-3 px-3 h-7 text-left text-[13px] rounded mx-1 row-hover focus-visible:bg-[var(--bg-3)] focus-visible:outline-none disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ width: 'calc(100% - 8px)', color: en.danger ? 'var(--danger)' : 'var(--text)' }}
                onClick={() => {
                  hide();
                  en.onSelect?.();
                }}
              >
                <span className="flex-1 truncate">{en.label}</span>
                {en.hint && <span className="mono text-[10.5px] truncate max-w-[140px]" style={{ color: 'var(--dim)' }}>{en.hint}</span>}
              </button>
            ),
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
};
