import React, { useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Apple, ChevronRight, Trash2 } from 'lucide-react';
import { useStore, sel } from '../store';
import { age, daysSince, splitBytes, bytes } from '../lib/format';
import { Icon } from '../lib/icons';
import { Checkbox } from '../components/ui/Checkbox';
import { itemProps } from '../lib/interact';
import { Tip } from '../components/ui/Tip';
import { AppStorageView } from './AppStorage';
import type { AppInfo } from '../lib/types';

type Sort = 'size' | 'used';

export const Apps: React.FC = () => {
  const apps = useStore((s) => s.apps);
  const [sort, setSort] = useState<Sort>('size');
  const [open, setOpen] = useState<AppInfo | null>(null);
  const sorted = useMemo(() => {
    const list = [...(apps ?? [])];
    return sort === 'size'
      ? list.sort((a, b) => b.app_bytes + b.data_bytes - (a.app_bytes + a.data_bytes))
      : list.sort((a, b) => (a.last_used ?? 0) - (b.last_used ?? 0));
  }, [apps, sort]);
  const total = sorted.reduce((a, x) => a + x.app_bytes + x.data_bytes, 0);
  const max = Math.max(1, ...sorted.map((a) => a.app_bytes + a.data_bytes));
  const t = splitBytes(total);
  const idle = sorted.filter((a) => daysSince(a.last_used) > 90).length;

  if (open) return <AppStorageView app={apps?.find((a) => a.path === open.path) ?? open} onBack={() => setOpen(null)} />;

  return (
    <div className="scroll h-full px-7 pb-32">
      <div className="flex items-end gap-6 pb-6 pt-1">
        {apps ? (
          <div>
            <div className="flex items-baseline gap-2">
              <span className="hero-num text-[64px] leading-[0.9]">{t.value}</span>
              <span className="num unit text-[24px]">{t.unit}</span>
            </div>
            <div className="text-[16px] font-medium muted mt-2.5">
              in {sorted.length} apps{idle > 0 && <span className="dim"> · {idle} unused for 3 months</span>}
            </div>
          </div>
        ) : (
          <div>
            <span className="h-[58px] w-[200px] rounded-2xl shimmer inline-block" />
            <div className="text-[16px] font-medium muted mt-2.5">Measuring your apps…</div>
          </div>
        )}
        <div className="flex-1" />
        <div className="no-drag seg rounded-full p-1 flex gap-0.5">
          {(['size', 'used'] as Sort[]).map((s) => (
            <button key={s} onClick={() => setSort(s)} className="relative h-8 px-4 rounded-full text-[13.5px] font-medium transition-colors" style={{ color: sort === s ? 'var(--text)' : 'var(--muted)' }}>
              {sort === s && <motion.span layoutId="apps-sort" className="absolute inset-0 rounded-full seg-on" transition={{ type: 'spring', stiffness: 420, damping: 40 }} />}
              <span className="relative">{s === 'size' ? 'Biggest' : 'Least used'}</span>
            </button>
          ))}
        </div>
      </div>
      {!apps && <div className="flex flex-col gap-3">{Array.from({ length: 8 }, (_, i) => <div key={i} className="h-[76px] rounded-3xl shimmer" />)}</div>}
      <div className="flex flex-col gap-1.5">
        <AnimatePresence initial={false}>
          {sorted.map((a, i) => <AppRow key={a.path} app={a} max={max} index={i} onOpen={a.storage ? () => setOpen(a) : undefined} />)}
        </AnimatePresence>
      </div>
    </div>
  );
};

const AppRow: React.FC<{ app: AppInfo; max: number; index: number; onOpen?: () => void }> = ({ app, max, index, onOpen }) => {
  const picked = useStore((s) => s.selection.has(`a:${app.path}`));
  const toggle = useStore((s) => s.toggle);
  const remove = useStore((s) => s.remove);
  const item = sel.app(app);
  const totalB = app.app_bytes + app.data_bytes;
  const s = splitBytes(totalB);
  const stale = daysSince(app.last_used) > 90;
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.9, x: -40, transition: { duration: 0.25 } }}
      transition={{ type: 'spring', stiffness: 320, damping: 32, delay: Math.min(0.3, index * 0.02) }}
    >
    {/* A row with a storage page opens it, like any row with a chevron; the checkbox selects. */}
    <div
      {...itemProps(item)}
      className="group flex items-center gap-5 px-4 py-3 rounded-[24px] row-hover"
      style={{ background: picked ? 'var(--accent-soft)' : undefined, cursor: onOpen ? 'pointer' : undefined }}
      onClick={() => (onOpen ? onOpen() : !app.running && toggle(item))}
    >
      <Checkbox checked={picked} disabled={app.running} onChange={() => toggle(item)} title={app.running ? `${app.name} is open` : undefined} />
      <div className="relative">
        <Icon iconKey={app.icon} size={56} />
        {app.running && <span className="absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 rounded-full" style={{ background: 'var(--ok)', boxShadow: '0 0 0 3px var(--bg)' }} />}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-[16px] font-medium truncate">{app.name}</span>
          {app.apple && <Apple size={15} className="muted shrink-0" />}
        </div>
        <Tip content={<span><b>{bytes(app.app_bytes)}</b> app · <b>{bytes(app.data_bytes)}</b> its data</span>} className="block">
          <div className="flex h-[5px] mt-2 max-w-[440px] gap-[2px]">
            <motion.div className="h-full rounded-l-full" style={{ background: 'var(--accent)' }} animate={{ width: `${(app.app_bytes / max) * 100}%` }} />
            <motion.div className="h-full rounded-r-full" style={{ background: 'color-mix(in srgb, var(--accent-text) 45%, transparent)' }} animate={{ width: `${(app.data_bytes / max) * 100}%` }} />
          </div>
        </Tip>
      </div>
      <div className="w-[130px] text-right shrink-0">
        <div className="text-[14px] font-medium" style={{ color: stale ? 'var(--warn)' : 'var(--muted)' }}>{app.running ? 'Open now' : app.last_used ? age(app.last_used) : 'Never opened'}</div>
      </div>
      <div className="w-[100px] text-right shrink-0">
        <span className="num font-semibold text-[19px]">{s.value}</span>
        <span className="num text-[14px] unit ml-1">{s.unit}</span>
      </div>
      <Tip content={app.running ? `Quit ${app.name} first` : `Move ${app.name} and its data to the Trash`}>
        <button className={`btn !h-10 ${app.running ? 'invisible group-hover:visible' : 'opacity-0 group-hover:opacity-100'}`} disabled={app.running} onClick={(e) => { e.stopPropagation(); remove([item]); }}>
          <Trash2 size={16} /> Uninstall
        </button>
      </Tip>
      {onOpen ? (
        <Tip content={`Everything ${app.name} keeps, item by item`}>
          <button className="btn btn-icon !w-10 !h-10" aria-label={`Manage ${app.name}`} onClick={(e) => { e.stopPropagation(); onOpen(); }}>
            <ChevronRight size={19} />
          </button>
        </Tip>
      ) : <span className="w-10 shrink-0" />}
    </div>
    </motion.div>
  );
};
