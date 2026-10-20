import React, { useEffect, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Sparkles, Trash2, LayoutGrid, Search, PieChart, History, LifeBuoy, RefreshCw, ScanSearch, Pin, PinOff, type LucideIcon } from 'lucide-react';
import { useStore } from '../store';
import { formatBytes, relativeTime } from '../lib/format';
import type { Page } from '../types';

type Entry = { id: Page; label: string; icon: LucideIcon; hint: string };

const SECTIONS: { title?: string; items: Entry[] }[] = [
  { items: [{ id: 'quick', label: 'Quick Clean', icon: Sparkles, hint: 'One click, no disk scan' }] },
  {
    title: 'Clean up',
    items: [
      { id: 'junk', label: 'Junk', icon: Trash2, hint: 'Everything rebuildable from your scan' },
      { id: 'apps', label: 'Apps', icon: LayoutGrid, hint: 'Space hiding inside apps' },
      { id: 'suggestions', label: 'Leftovers', icon: Search, hint: 'Uninstalled apps, duplicates, AI picks' },
    ],
  },
  {
    title: 'Explore',
    items: [
      { id: 'explore', label: 'Space map', icon: PieChart, hint: 'Where the space went' },
      { id: 'timeline', label: 'Timeline', icon: History, hint: 'What you haven’t touched in ages' },
    ],
  },
  { items: [{ id: 'quarantine', label: 'Quarantine', icon: LifeBuoy, hint: 'Restore or empty' }] },
];

export const RAIL = 76;
export const RAIL_OPEN = 236;

/** A slim rail of app icons. Hover opens it over the content; the pin keeps it open. */
export const Sidebar: React.FC<{ onScan: () => void; scanning: boolean; pinned: boolean; onPin: (v: boolean) => void }> = ({ onScan, scanning, pinned, onPin }) => {
  const page = useStore((s) => s.page);
  const startScreen = useStore((s) => s.startScreen);
  const setPage = useStore((s) => s.setPage);
  const setStartScreen = useStore((s) => s.setStartScreen);
  const journals = useStore((s) => s.journals);
  const vol = useStore((s) => s.overview?.volume ?? s.info?.volume);
  const scan = useStore((s) => s.info?.scan);
  const held = journals.filter((j) => j.items.some((i) => i.present)).length;
  const [hover, setHover] = useState(false);
  const timer = useRef<number>();
  const open = pinned || hover;
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const enter = () => { window.clearTimeout(timer.current); timer.current = window.setTimeout(() => setHover(true), 140); };
  const leave = () => { window.clearTimeout(timer.current); timer.current = window.setTimeout(() => setHover(false), 120); };

  return (
    <motion.nav
      className="dock"
      data-open={open}
      data-floating={open && !pinned}
      aria-label="Sections"
      onMouseEnter={enter}
      onMouseLeave={leave}
      animate={{ width: open ? RAIL_OPEN : RAIL }}
      transition={{ type: 'spring', stiffness: 420, damping: 38 }}
    >
      <div className="h-[52px] flex-shrink-0 drag-region" />
      <div className="dock-brand">
        <svg className="dock-logo" viewBox="0 0 36 36" aria-hidden="true">
          <circle cx="18" cy="18" r="11" fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
          <path d="M18 7 A11 11 0 1 1 7.5 21.3" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
        </svg>
        <AnimatePresence>{open && <motion.span initial={{ opacity: 0, x: -6 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }} className="dock-brand-name">Disko</motion.span>}</AnimatePresence>
        <AnimatePresence>
          {open && (
            <motion.button initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="dock-pin" title={pinned ? 'Unpin sidebar (⌘\\)' : 'Keep sidebar open (⌘\\)'} onClick={() => onPin(!pinned)}>
              {pinned ? <PinOff className="w-3.5 h-3.5" /> : <Pin className="w-3.5 h-3.5" />}
            </motion.button>
          )}
        </AnimatePresence>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden px-3 pb-3" style={{ scrollbarWidth: 'none' }}>
        {SECTIONS.map((sec, i) => (
          <div key={i} className={i ? 'mt-3' : 'mt-1'}>
            <div className="dock-title" style={{ opacity: open && sec.title ? 1 : 0 }}>{sec.title ?? ''}</div>
            {sec.items.map((e) => {
              const active = page === e.id && !startScreen;
              const Icon = e.icon;
              return (
                <button key={e.id} className="dock-item" aria-current={active ? 'page' : undefined} title={open ? e.hint : e.label} onClick={() => { setPage(e.id); setStartScreen(false); }}>
                  {active && <motion.span layoutId="dock-active" className="dock-active" transition={{ type: 'spring', stiffness: 520, damping: 40 }} />}
                  <span className="dock-icon">
                    <Icon className="w-[18px] h-[18px]" strokeWidth={active ? 2.1 : 1.7} />
                    {e.id === 'quarantine' && held > 0 && <span className="dock-badge tnum">{held}</span>}
                  </span>
                  <span className="dock-label">{e.label}</span>
                </button>
              );
            })}
          </div>
        ))}
      </div>

      <div className="dock-foot">
        <button className="dock-disk" onClick={onScan} disabled={scanning} aria-pressed={startScreen} title={scanning ? 'Scanning…' : 'Disk scan'}>
          <DiskRing used={vol?.total ? 1 - vol.available / vol.total : 0} spinning={scanning} />
          <span className="dock-label flex-col !items-start">
            <span className="text-[12.5px] font-semibold tnum">{vol?.total ? `${formatBytes(vol.available)} free` : 'Disk'}</span>
            <span className="text-[10.5px] font-normal flex items-center gap-1" style={{ color: 'var(--dim)' }}>
              {scanning ? <><RefreshCw className="w-2.5 h-2.5 animate-spin" /> Scanning…</> : <><ScanSearch className="w-2.5 h-2.5" /> Disk scan · {scan?.finished ? relativeTime(scan.finished) : 'never'}</>}
            </span>
          </span>
        </button>
      </div>
    </motion.nav>
  );
};

function DiskRing({ used, spinning }: { used: number; spinning: boolean }) {
  const r = 15, c = 2 * Math.PI * r;
  return (
    <span className="dock-ring">
      <svg width="38" height="38" viewBox="0 0 38 38" style={{ transform: 'rotate(-90deg)' }}>
        <circle cx="19" cy="19" r={r} fill="none" stroke="var(--line-2)" strokeWidth="2.5" />
        <motion.circle cx="19" cy="19" r={r} fill="none" stroke={used > 0.9 ? 'var(--danger)' : 'var(--text)'} strokeWidth="2.5" strokeLinecap="round" strokeDasharray={c}
          initial={false} animate={{ strokeDashoffset: c * (1 - used) }} transition={{ duration: 1, ease: [0.16, 1, 0.3, 1] }} />
      </svg>
      <span className="dock-ring-label tnum">{spinning ? <RefreshCw className="w-3 h-3 animate-spin" /> : `${Math.round(used * 100)}`}</span>
    </span>
  );
}
