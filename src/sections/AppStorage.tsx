import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronLeft, Lock, Repeat, Sparkles, Terminal, Trash2, X } from 'lucide-react';
import { useStore, sel } from '../store';
import { api } from '../lib/api';
import { age, splitBytes } from '../lib/format';
import { Glyph, Icon } from '../lib/icons';
import { Checkbox } from '../components/ui/Checkbox';
import { itemProps } from '../lib/interact';
import { Tip } from '../components/ui/Tip';
import type { AppInfo, AppStorage, Finding, StorageGroup } from '../lib/types';

type Sort = 'size' | 'used';

/**
 * One app's storage page inside the Apps section: every simulator, runtime, cache … as rows the
 * user picks from. Rows are findings, so the global selection and SelectionBar remove them the
 * same way as clutter: one tap, Trash + Undo for anything that isn't rebuildable.
 */
export const AppStorageView: React.FC<{ app: AppInfo; onBack: () => void }> = ({ app, onBack }) => {
  const [data, setData] = useState<AppStorage | null | undefined>(undefined);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [sort, setSort] = useState<Sort>('size');
  const remove = useStore((s) => s.remove);

  const load = useCallback(() => {
    api.appStorage(app.path).then((d) => { setData(d); setHidden(new Set()); }).catch(() => setData((d) => d ?? null));
  }, [app.path]);
  useEffect(() => { setData(undefined); load(); }, [load]);

  // Rows vanish the moment a removal starts (from the SelectionBar or a row's own button), and the
  // page reloads when it's done so sizes are true and anything that stayed comes back.
  useEffect(() => useStore.subscribe((s, prev) => {
    if (s.busyRemoving && !prev.busyRemoving) {
      const gone = [...prev.selection.keys()].filter((k) => k.startsWith('f:') && !s.selection.has(k)).map((k) => k.slice(2));
      if (gone.length) setHidden((h) => new Set([...h, ...gone]));
    }
    if (!s.busyRemoving && prev.busyRemoving) load();
  }), [load]);

  const hide = (ids: string[]) => setHidden((h) => new Set([...h, ...ids]));
  const removeNow = (items: Finding[]) => {
    const todo = items.filter((f) => !f.busy);
    hide(todo.map((f) => f.id));
    remove(todo.map(sel.finding));
  };

  const groups = useMemo(() => {
    if (!data) return [];
    const order = (a: Finding, b: Finding) => sort === 'size'
      ? b.bytes - a.bytes
      : (!!a.busy === !!b.busy ? (a.last_used ?? 0) - (b.last_used ?? 0) : a.busy ? 1 : -1);
    return data.groups
      .map((g) => {
        const items = g.items.filter((f) => !hidden.has(f.id));
        return { ...g, items: g.id === 'simulators' && sort === 'size' ? items : [...items].sort(order), bytes: items.reduce((a, f) => a + f.bytes, 0) };
      })
      .filter((g) => g.items.length > 0);
  }, [data, hidden, sort]);
  const total = groups.reduce((a, g) => a + g.bytes, 0);
  const count = groups.reduce((a, g) => a + g.items.length, 0);
  const t = splitBytes(total);
  const safe = groups.flatMap((g) => g.items).filter((f) => f.safety === 'safe' && !f.busy);
  const safeBytes = safe.reduce((a, f) => a + f.bytes, 0);

  return (
    <div className="scroll h-full px-7 pb-32">
      <button className="no-drag btn btn-ghost !h-9 !px-3 -ml-3 mb-2 !text-[14px] muted" onClick={onBack}>
        <ChevronLeft size={18} /> Apps
      </button>

      {/* Hero: the app and what it keeps */}
      <div className="flex items-end gap-6 pb-7 flex-wrap">
        <Icon iconKey={app.icon} size={88} />
        <div className="min-w-0">
          <div className="text-[17px] font-medium muted truncate">{app.name}{data?.app.version && <span className="dim"> {data.app.version}</span>}</div>
          <div className="flex items-baseline gap-2 mt-1">
            {data === undefined
              ? <span className="h-[58px] w-[180px] rounded-2xl shimmer inline-block" />
              : <><span className="hero-num text-[64px] leading-[0.9]">{t.value}</span><span className="num unit text-[24px]">{t.unit}</span></>}
          </div>
          {data !== undefined && <div className="text-[15px] font-medium dim mt-2">{count} {count === 1 ? 'item' : 'items'}</div>}
        </div>
        <div className="flex-1" />
        {data && (
          <div className="flex items-center gap-3">
            {safe.length > 1 && (
              <button className="btn !h-10" onClick={() => removeNow(safe)}>
                <Sparkles size={16} /> Clean rebuildable · <span className="num">{splitBytes(safeBytes).value} {splitBytes(safeBytes).unit}</span>
              </button>
            )}
            <div className="no-drag seg rounded-full p-1 flex gap-0.5">
              {(['size', 'used'] as Sort[]).map((s) => (
                <button key={s} onClick={() => setSort(s)} className="relative h-8 px-4 rounded-full text-[13.5px] font-medium transition-colors" style={{ color: sort === s ? 'var(--text)' : 'var(--muted)' }}>
                  {sort === s && <motion.span layoutId="storage-sort" className="absolute inset-0 rounded-full seg-on" transition={{ type: 'spring', stiffness: 420, damping: 40 }} />}
                  <span className="relative">{s === 'size' ? 'Biggest' : 'Least used'}</span>
                </button>
              ))}
            </div>
          </div>
        )}
      </div>

      {data === undefined && <div className="flex flex-col gap-3">{Array.from({ length: 6 }, (_, i) => <div key={i} className="h-[64px] rounded-3xl shimmer" />)}</div>}
      {data === null && <div className="muted text-[16px] py-16 text-center">Couldn’t read what {app.name} keeps.</div>}
      {data && !groups.length && (
        <div className="flex flex-col items-center justify-center py-20 text-center">
          <Sparkles size={44} strokeWidth={1.4} style={{ color: 'var(--accent-text)' }} />
          <div className="text-[26px] font-semibold tracking-[-0.03em] mt-4">Nothing to sort out</div>
        </div>
      )}

      <div className="flex flex-col gap-4">
        <AnimatePresence initial={false}>
          {groups.map((g) => <GroupCard key={g.id} g={g} onRemove={removeNow} />)}
        </AnimatePresence>
      </div>
    </div>
  );
};

const GroupCard: React.FC<{ g: StorageGroup; onRemove: (items: Finding[]) => void }> = ({ g, onRemove }) => {
  const selection = useStore((s) => s.selection);
  const setMany = useStore((s) => s.setMany);
  const all = g.items.filter((f) => !f.busy).map(sel.finding);
  const picked = all.filter((i) => selection.has(i.key)).length;
  const s = splitBytes(g.bytes);
  return (
    <motion.div layout initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, scale: 0.96 }} className="glass rounded-[26px] overflow-hidden">
      <div className="flex items-center gap-4 px-5 pt-5 pb-3">
        <Glyph name={g.icon} size={44} />
        <div className="flex-1 min-w-0">
          <div className="text-[15px] font-medium muted truncate">{g.title}<span className="dim"> · {g.items.length}</span></div>
          <div className="flex items-baseline gap-1.5 mt-0.5">
            <span className="num text-[26px] font-semibold leading-none tracking-[-0.035em]">{s.value}</span>
            <span className="num text-[14px] unit">{s.unit}</span>
          </div>
        </div>
        {all.length > 1 && (
          <label className="flex items-center gap-2.5 cursor-pointer select-none" onClick={() => setMany(all, picked !== all.length)}>
            <span className="muted text-[13.5px]">{picked ? `${picked} selected` : 'Select all'}</span>
            <Checkbox checked={picked === all.length} mixed={picked > 0 && picked < all.length} onChange={(v) => setMany(all, v)} />
          </label>
        )}
      </div>
      {g.note && <div className="dim text-[13px] px-5 -mt-1 pb-2">{g.note}</div>}
      <div className="px-2 pb-3">
        <AnimatePresence initial={false}>
          {g.items.map((f) => <ItemRow key={f.id} f={f} onRemove={onRemove} />)}
        </AnimatePresence>
      </div>
    </motion.div>
  );
};

const ItemRow: React.FC<{ f: Finding; onRemove: (items: Finding[]) => void }> = ({ f, onRemove }) => {
  const picked = useStore((s) => s.selection.has(`f:${f.id}`));
  const toggle = useStore((s) => s.toggle);
  const item = sel.finding(f);
  const s = splitBytes(f.bytes);
  const row = (
    <div
      {...itemProps(item)}
      className="group flex items-center gap-4 px-3 py-2.5 rounded-2xl row-hover"
      style={{ opacity: f.busy ? 0.5 : 1, background: picked ? 'var(--accent-soft)' : undefined }}
      onClick={() => !f.busy && toggle(item)}
    >
      <Checkbox checked={picked} disabled={!!f.busy} onChange={() => toggle(item)} />
      <Icon iconKey={f.icon} size={40} />
      <div className="flex-1 min-w-0">
        <div className="text-[15.5px] font-medium truncate">{f.title}</div>
        {(f.busy || f.detail) && (
          <div className="text-[13px] truncate mt-0.5" style={{ color: f.busy ? 'var(--warn)' : 'var(--muted)' }}>{f.busy ?? f.detail}</div>
        )}
      </div>
      <div className="flex items-center gap-2.5 dim shrink-0">
        {f.action.type === 'command' && <Tip content={<>Removed with <b>{f.action.label}</b>, not by touching its files</>}><Terminal size={15} /></Tip>}
        {f.regrows && <Tip content="Comes back by itself"><Repeat size={15} /></Tip>}
        {f.admin && <Tip content="Needs your Mac password"><Lock size={15} /></Tip>}
        {f.safety === 'review' && f.action.type === 'trash' && <Tip content="Goes to the Trash, so you can put it back"><Trash2 size={15} /></Tip>}
      </div>
      <span className="muted text-[13px] w-[96px] text-right shrink-0">{age(f.last_used)}</span>
      <div className="w-[96px] text-right shrink-0">
        <span className="num font-semibold text-[18px]">{s.value}</span>
        <span className="num text-[13px] unit ml-1">{s.unit}</span>
      </div>
      <button className={`btn btn-icon !w-9 !h-9 ${f.busy ? 'invisible' : 'opacity-0 group-hover:opacity-100'}`} aria-label="Remove" disabled={!!f.busy} onClick={(e) => { e.stopPropagation(); onRemove([f]); }}>
        <X size={17} />
      </button>
    </div>
  );
  return (
    <motion.div layout initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, x: 60, height: 0, transition: { duration: 0.22 } }}>
      {row}
    </motion.div>
  );
};
