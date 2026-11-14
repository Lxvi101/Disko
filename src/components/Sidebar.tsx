import React from 'react';
import { motion } from 'framer-motion';
import { Search, PieChart, LifeBuoy, LayoutGrid, Trash2, Sparkles, History, RefreshCw, type LucideIcon } from 'lucide-react';
import { useStore } from '../store';
import { formatBytes, relativeTime } from '../lib/format';
import type { Page } from '../types';

type Entry = { id: Page; label: string; icon: LucideIcon; hint: string };

const MAIN: Entry[] = [
  { id: 'quick', label: 'Find space', icon: Search, hint: 'Ask, pick, clean' },
  { id: 'explore', label: 'Space map', icon: PieChart, hint: 'Where the space went' },
  { id: 'quarantine', label: 'Quarantine', icon: LifeBuoy, hint: 'Restore or empty' },
];
const MORE: Entry[] = [
  { id: 'apps', label: 'Apps & Xcode', icon: LayoutGrid, hint: 'Space hiding inside apps' },
  { id: 'junk', label: 'All junk', icon: Trash2, hint: 'Everything rebuildable from your disk scan' },
  { id: 'suggestions', label: 'Suggestions', icon: Sparkles, hint: 'AI picks, duplicates, leftovers of removed apps' },
  { id: 'timeline', label: 'Timeline', icon: History, hint: 'What you haven’t touched in ages' },
];

export const SIDEBAR = 212;

export const Sidebar: React.FC<{ onScan: () => void; scanning: boolean }> = ({ onScan, scanning }) => {
  const page = useStore((s) => s.page);
  const startScreen = useStore((s) => s.startScreen);
  const setPage = useStore((s) => s.setPage);
  const setStartScreen = useStore((s) => s.setStartScreen);
  const journals = useStore((s) => s.journals);
  const vol = useStore((s) => s.overview?.volume ?? s.info?.volume);
  const scan = useStore((s) => s.info?.scan);
  const plan = useStore((s) => s.planBytes);
  const held = journals.filter((j) => j.items.some((i) => i.present)).length;
  const used = vol?.total ? 1 - vol.available / vol.total : 0;
  const preview = vol?.total ? Math.min(used, plan / vol.total) : 0;

  const item = (e: Entry, small?: boolean) => {
    const active = page === e.id && !startScreen;
    const Icon = e.icon;
    return (
      <button key={e.id} className={`side-item ${small ? 'side-item-small' : ''}`} aria-current={active ? 'page' : undefined} title={e.hint} onClick={() => { setPage(e.id); setStartScreen(false); }}>
        {active && <motion.span layoutId="side-active" className="side-active" transition={{ type: 'spring', stiffness: 520, damping: 40 }} />}
        <Icon className="side-icon" strokeWidth={active ? 2.1 : 1.8} />
        <span className="side-label">{e.label}</span>
        {e.id === 'quarantine' && held > 0 && <span className="side-count tnum">{held}</span>}
      </button>
    );
  };

  return (
    <nav className="side" aria-label="Sections" style={{ width: SIDEBAR }}>
      <div className="h-[44px] flex-shrink-0 drag-region" />
      <div className="side-brand drag-region">
        <svg width="22" height="22" viewBox="0 0 36 36" aria-hidden="true">
          <circle cx="18" cy="18" r="11" fill="none" stroke="var(--fs-accent)" strokeOpacity="0.25" strokeWidth="3.5" />
          <path d="M18 7A11 11 0 1 1 7.5 21.3" fill="none" stroke="var(--fs-accent)" strokeWidth="3.5" strokeLinecap="round" />
        </svg>
        Disko
      </div>
      <div className="flex-1 overflow-y-auto px-3" style={{ scrollbarWidth: 'none' }}>
        {MAIN.map((e) => item(e))}
        <div className="side-sec">More tools</div>
        {MORE.map((e) => item(e, true))}
      </div>
      <div className="p-3">
        <button className="side-disk" onClick={onScan} disabled={scanning} aria-pressed={startScreen} title={scanning ? 'Scanning…' : 'Scan your disk again to refresh the map and every list'}>
          <span className="side-disk-top"><b className="tnum">{vol?.total ? `${formatBytes(vol.available)} free` : 'Disk'}</b>{vol?.total ? <span className="tnum">of {formatBytes(vol.total)}</span> : null}</span>
          <span className="side-bar">
            <motion.span className="side-bar-used" data-low={used > 0.9} initial={false} animate={{ width: `${(used - preview) * 100}%` }} transition={{ duration: 0.8, ease: [0.16, 1, 0.3, 1] }} />
            <motion.span className="side-bar-plan" initial={false} animate={{ width: `${preview * 100}%` }} transition={{ duration: 0.8, ease: [0.16, 1, 0.3, 1] }} />
          </span>
          <span className="side-disk-note">
            {scanning ? <><RefreshCw className="w-2.5 h-2.5 animate-spin" /> Scanning…</>
              : plan > 0 && vol?.total ? `${formatBytes(vol.available + plan)} free after this`
              : scan?.finished ? `Scan again · ${relativeTime(scan.finished)}` : 'Scan your disk'}
          </span>
        </button>
      </div>
    </nav>
  );
};
