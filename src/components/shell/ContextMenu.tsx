import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CheckCircle2, Circle, Eye, FolderOpen, Orbit, Shield, Trash2, type LucideIcon } from 'lucide-react';
import { useStore } from '../../store';
import { api } from '../../lib/api';
import { bytes } from '../../lib/format';

/** One right-click menu for every item in the app. */
export const ContextMenu: React.FC = () => {
  const menu = useStore((s) => s.menu);
  const close = useStore((s) => s.closeMenu);
  const selection = useStore((s) => s.selection);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useEffect(() => {
    if (!menu) return;
    const off = (e: Event) => { if (!ref.current?.contains(e.target as Node)) close(); };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') close(); };
    window.addEventListener('mousedown', off, true);
    window.addEventListener('wheel', close, { passive: true });
    window.addEventListener('blur', close);
    window.addEventListener('keydown', key);
    return () => {
      window.removeEventListener('mousedown', off, true);
      window.removeEventListener('wheel', close);
      window.removeEventListener('blur', close);
      window.removeEventListener('keydown', key);
    };
  }, [menu, close]);

  // Keep it on screen.
  useLayoutEffect(() => {
    if (!menu || !ref.current) return setPos(null);
    const r = ref.current.getBoundingClientRect();
    setPos({ left: Math.min(menu.x, window.innerWidth - r.width - 8), top: Math.min(menu.y, window.innerHeight - r.height - 8) });
  }, [menu]);

  const items = menu?.items ?? [];
  const allPicked = items.length > 0 && items.every((i) => selection.has(i.key));
  const locked = items.find((i) => i.locked)?.locked;
  const busy = locked ?? items.find((i) => i.busy)?.busy;
  const permanent = items.some((i) => i.permanent);
  const allTrash = items.every((i) => i.trash);
  const many = items.length > 1;
  const run = (fn: () => void) => () => { close(); fn(); };
  const st = useStore.getState;

  return (
    <AnimatePresence>
      {menu && (
        <motion.div
          ref={ref}
          key={`${menu.x},${menu.y}`}
          initial={{ opacity: 0, scale: 0.96 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, transition: { duration: 0.08 } }}
          transition={{ duration: 0.12 }}
          className="fixed z-[90] glass-strong rounded-2xl p-1.5 min-w-[220px] origin-top-left"
          style={{ left: pos?.left ?? menu.x, top: pos?.top ?? menu.y, visibility: pos ? 'visible' : 'hidden' }}
          onContextMenu={(e) => e.preventDefault()}
        >
          <div className="px-3 pt-1.5 pb-2 flex items-baseline gap-2 min-w-0">
            <span className="text-[13px] font-semibold truncate max-w-[220px]">{many ? `${items.length} items` : items[0]?.title}</span>
            <span className="num text-[12.5px] muted ml-auto">{bytes(items.reduce((a, i) => a + i.bytes, 0))}</span>
          </div>
          {!many && menu.folder && <Item icon={Orbit} label="Open in map" onClick={run(() => st().openFolder(menu.folder!))} />}
          {!many && menu.path && <Item icon={FolderOpen} label="Show in Finder" onClick={run(() => api.reveal(menu.path!))} />}
          {!many && menu.path && <Item icon={Eye} label="Quick Look" hint="Space" onClick={run(() => api.quicklook(menu.path!))} />}
          {!many && (menu.folder || menu.path) && <div className="h-px my-1 mx-2" style={{ background: 'var(--line)' }} />}
          {!locked && <Item icon={allPicked ? Circle : CheckCircle2} label={allPicked ? 'Deselect' : 'Select'} onClick={run(() => st().setMany(items, !allPicked))} />}
          {!locked && <Item icon={Shield} label="Always keep" onClick={run(() => st().keep(items))} />}
          <Item
            icon={Trash2}
            label={allTrash ? 'Move to Trash' : 'Remove'}
            danger
            disabled={busy ?? (permanent ? 'Use Hold to empty in Clutter' : undefined)}
            onClick={run(() => st().remove(items))}
          />
        </motion.div>
      )}
    </AnimatePresence>
  );
};

const Item: React.FC<{ icon: LucideIcon; label: string; onClick: () => void; danger?: boolean; disabled?: string; hint?: string }> = ({ icon: I, label, onClick, danger, disabled, hint }) => (
  <button
    disabled={!!disabled}
    onClick={onClick}
    className="w-full flex items-center gap-3 h-9 px-3 rounded-xl text-left text-[14px] font-medium row-hover disabled:opacity-40 disabled:pointer-events-none"
    style={{ color: danger ? 'var(--danger)' : 'var(--text)' }}
  >
    <I size={16} strokeWidth={1.8} className="shrink-0" />
    <span className="flex-1 truncate">{label}</span>
    {(disabled || hint) && <span className="text-[12px] muted truncate max-w-[140px]">{disabled ?? hint}</span>}
  </button>
);
