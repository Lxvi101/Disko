import React, { useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Apple, Trash2 } from 'lucide-react';
import { useStore, sel } from '../store';
import { age, daysSince, splitBytes, bytes } from '../lib/format';
import { Icon } from '../lib/icons';
import { Checkbox } from '../components/ui/Checkbox';
import { Tip } from '../components/ui/Tip';
import type { AppInfo } from '../lib/types';

type Sort = 'size' | 'used';

export const Apps: React.FC = () => {
  const apps = useStore((s) => s.apps);
  const [sort, setSort] = useState<Sort>('size');
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

  return (
    <div className="scroll h-full px-7 pb-32">
      <div className="flex items-end gap-6 pb-6 pt-1">
        <div>
          <div className="flex items-baseline gap-2">
            <span className="num text-[56px] font-bold leading-none">{t.value}</span>
            <span className="num text-[24px] font-bold muted">{t.unit}</span>
          </div>
          <div className="text-[18px] font-semibold mt-1.5">
            in {sorted.length} apps{idle > 0 && <span className="muted font-medium"> · {idle} unused for 3 months</span>}
          </div>
        </div>
        <div className="flex-1" />
        <div className="no-drag glass rounded-full p-1 flex gap-1">
          {(['size', 'used'] as Sort[]).map((s) => (
            <button key={s} onClick={() => setSort(s)} className="relative h-9 px-4 rounded-full text-[14px] font-semibold" style={{ color: sort === s ? 'white' : 'var(--muted)' }}>
              {sort === s && <motion.span layoutId="apps-sort" className="absolute inset-0 rounded-full" style={{ background: 'linear-gradient(135deg, #9a7bff, #5b6cff)' }} />}
              <span className="relative">{s === 'size' ? 'Biggest' : 'Least used'}</span>
            </button>
          ))}
        </div>
      </div>
      {!apps && <div className="flex flex-col gap-3">{Array.from({ length: 8 }, (_, i) => <div key={i} className="h-[76px] rounded-3xl shimmer" />)}</div>}
      <div className="flex flex-col gap-1.5">
        <AnimatePresence initial={false}>
          {sorted.map((a, i) => <AppRow key={a.path} app={a} max={max} index={i} />)}
        </AnimatePresence>
      </div>
    </div>
  );
};

const AppRow: React.FC<{ app: AppInfo; max: number; index: number }> = ({ app, max, index }) => {
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
      className="group flex items-center gap-5 px-4 py-3 rounded-[24px] row-hover"
      style={{ background: picked ? 'color-mix(in srgb, var(--accent) 10%, transparent)' : undefined }}
      onClick={() => !app.running && toggle(item)}
    >
      <Checkbox checked={picked} disabled={app.running} onChange={() => toggle(item)} title={app.running ? `${app.name} is open` : undefined} />
      <div className="relative">
        <Icon iconKey={app.icon} size={56} />
        {app.running && <span className="absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 rounded-full" style={{ background: 'var(--ok)', boxShadow: '0 0 0 3px var(--bg), 0 0 10px var(--ok)' }} />}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-[18px] font-semibold truncate">{app.name}</span>
          {app.apple && <Apple size={15} className="muted shrink-0" />}
        </div>
        <Tip content={<span><b>{bytes(app.app_bytes)}</b> app · <b>{bytes(app.data_bytes)}</b> its data</span>} className="block">
          <div className="flex h-2 mt-2 max-w-[440px] bar-track">
            <motion.div className="h-full" style={{ background: 'linear-gradient(90deg,#9a7bff,#5b6cff)' }} animate={{ width: `${(app.app_bytes / max) * 100}%` }} />
            <motion.div className="h-full" style={{ background: 'linear-gradient(90deg,#4cc9ff,#2fd3e6)' }} animate={{ width: `${(app.data_bytes / max) * 100}%` }} />
          </div>
        </Tip>
      </div>
      <div className="w-[130px] text-right shrink-0">
        <div className="text-[14px] font-medium" style={{ color: stale ? '#ffb36b' : 'var(--muted)' }}>{app.running ? 'Open now' : app.last_used ? age(app.last_used) : 'Never opened'}</div>
      </div>
      <div className="w-[100px] text-right shrink-0">
        <span className="num font-bold text-[20px]">{s.value}</span>
        <span className="num font-semibold text-[14px] muted ml-1">{s.unit}</span>
      </div>
      <Tip content={app.running ? `Quit ${app.name} first` : `Move ${app.name} and its data to the Trash`}>
        <button className={`btn !h-10 ${app.running ? 'invisible group-hover:visible' : 'opacity-0 group-hover:opacity-100'}`} disabled={app.running} onClick={(e) => { e.stopPropagation(); remove([item]); }}>
          <Trash2 size={16} /> Uninstall
        </button>
      </Tip>
    </motion.div>
  );
};
