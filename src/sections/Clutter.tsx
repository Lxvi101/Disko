import React, { useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Clock, Lock, Repeat, Sparkles, Terminal, Trash2, X, ChevronDown } from 'lucide-react';
import { useStore, sel } from '../store';
import { api } from '../lib/api';
import { CATEGORIES, type CategoryMeta } from '../lib/categories';
import { age, splitBytes, bytes } from '../lib/format';
import { Icon } from '../lib/icons';
import { Checkbox } from '../components/ui/Checkbox';
import { HoldButton } from '../components/ui/HoldButton';
import { Tip } from '../components/ui/Tip';
import type { Category, Finding, Routine } from '../lib/types';

export const Clutter: React.FC = () => {
  const findings = useStore((s) => s.findings);
  const progress = useStore((s) => s.progress);
  const remove = useStore((s) => s.remove);
  const [open, setOpen] = useState<Category | null>(null);
  const list = Object.values(findings);
  const total = list.reduce((a, f) => a + f.bytes, 0);
  const safe = list.filter((f) => f.safety === 'safe' && !f.busy && f.category !== 'trash');
  const safeBytes = safe.reduce((a, f) => a + f.bytes, 0);
  const scanning = progress && progress.phase !== 'done';
  const groups = useMemo(() => {
    const m = new Map<Category, Finding[]>();
    for (const f of list) m.set(f.category, [...(m.get(f.category) ?? []), f]);
    return CATEGORIES.filter((c) => m.has(c.id))
      .map((c) => ({ meta: c, items: m.get(c.id)!.sort((a, b) => b.bytes - a.bytes), bytes: m.get(c.id)!.reduce((a, f) => a + f.bytes, 0) }))
      .sort((a, b) => (a.meta.id === 'trash' ? 1 : b.meta.id === 'trash' ? -1 : b.bytes - a.bytes));
  }, [list]);
  const t = splitBytes(total);

  return (
    <div className="scroll h-full px-7 pb-32">
      {/* Hero */}
      <div className="flex items-end gap-8 pt-2 pb-6 flex-wrap">
        <div>
          <div className="flex items-baseline gap-3">
            <motion.span key={t.value} initial={{ opacity: 0.6 }} animate={{ opacity: 1 }} className="num font-bold leading-[0.9]" style={{ fontSize: 'clamp(64px, 8vw, 104px)', background: 'linear-gradient(120deg,#ffb36b,#ff5fa2 45%,#a98bff)', WebkitBackgroundClip: 'text', color: 'transparent' }}>
              {t.value}
            </motion.span>
            <span className="num font-bold text-[34px]" style={{ color: '#ff8fb8' }}>{t.unit}</span>
          </div>
          <div className="text-[22px] font-semibold mt-2 flex items-center gap-3">
            of clutter
            {scanning && <span className="muted text-[15px] font-medium flex items-center gap-2"><span className="w-2 h-2 rounded-full" style={{ background: '#7cc8ff', animation: 'spin 1s linear infinite', boxShadow: '0 0 10px #7cc8ff' }} />still looking</span>}
          </div>
        </div>
        <div className="flex-1" />
        {safe.length > 0 && (
          <button className="btn btn-primary !h-14 !px-7 !text-[17px]" onClick={() => remove(safe.map(sel.finding))}>
            <Sparkles size={20} /> Clean all safe · <span className="num">{bytes(safeBytes)}</span>
          </button>
        )}
      </div>

      {/* Proportions */}
      {total > 0 && (
        <div className="flex h-3 rounded-full overflow-hidden gap-[3px] mb-7">
          {groups.map((g) => (
            <motion.div key={g.meta.id} layout className="h-full rounded-full" style={{ background: `linear-gradient(90deg, ${g.meta.color}, ${g.meta.color2})`, flexGrow: g.bytes, flexBasis: 0, minWidth: 6 }} />
          ))}
        </div>
      )}

      {!list.length && !scanning && <FactoryFresh />}

      <motion.div layout className="grid gap-4" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(290px, 1fr))' }}>
        <AnimatePresence initial={false}>
          {groups.map((g) => (
            <CategoryCard key={g.meta.id} meta={g.meta} items={g.items} total={g.bytes} open={open === g.meta.id} onToggle={() => setOpen(open === g.meta.id ? null : g.meta.id)} />
          ))}
        </AnimatePresence>
      </motion.div>
    </div>
  );
};

const FactoryFresh = () => (
  <div className="flex flex-col items-center justify-center py-24 text-center">
    <Sparkles size={56} style={{ color: '#a98bff' }} />
    <div className="text-[32px] font-bold mt-4">Factory fresh</div>
    <div className="muted text-[16px] mt-1">Nothing left to clean.</div>
  </div>
);

const CategoryCard: React.FC<{ meta: CategoryMeta; items: Finding[]; total: number; open: boolean; onToggle: () => void }> = ({ meta, items, total, open, onToggle }) => {
  const selection = useStore((s) => s.selection);
  const setMany = useStore((s) => s.setMany);
  const remove = useStore((s) => s.remove);
  const s = splitBytes(total);
  const regrows = items.every((f) => f.regrows);
  const all = items.filter((f) => !f.busy).map(sel.finding);
  const picked = all.filter((i) => selection.has(i.key)).length;
  const I = meta.icon;
  return (
    <motion.div
      layout
      initial={{ opacity: 0, scale: 0.92, y: 16 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.9 }}
      transition={{ type: 'spring', stiffness: 300, damping: 30 }}
      className="glass rounded-[28px] overflow-hidden relative"
      style={{ gridColumn: open ? '1 / -1' : undefined, boxShadow: open ? `0 0 0 1.5px ${meta.color}66, 0 30px 60px -30px ${meta.color}88` : undefined }}
    >
      <div className="absolute -top-16 -right-10 w-48 h-48 rounded-full pointer-events-none" style={{ background: `radial-gradient(circle, ${meta.color}40, transparent 70%)` }} />
      <motion.div layout="position" className="relative flex items-start gap-4 p-5 cursor-pointer" onClick={onToggle}>
        <span className="w-14 h-14 rounded-[18px] flex items-center justify-center shrink-0" style={{ background: `linear-gradient(145deg, ${meta.color}, ${meta.color2})`, boxShadow: `0 10px 24px -10px ${meta.color}, inset 0 1px 0 rgba(255,255,255,.35)` }}>
          <I size={28} color="white" strokeWidth={2} />
        </span>
        <div className="flex-1 min-w-0">
          <div className="text-[17px] font-semibold flex items-center gap-2">
            <span className="truncate">{meta.label}</span>
            {regrows && <Tip content="Comes back by itself"><Repeat size={15} className="muted" /></Tip>}
          </div>
          <div className="flex items-baseline gap-1.5 mt-0.5">
            <span className="num text-[30px] font-bold leading-none">{s.value}</span>
            <span className="num text-[16px] font-semibold muted">{s.unit}</span>
          </div>
          {open && <div className="muted text-[14px] mt-2">{meta.blurb}</div>}
        </div>
        <div className="flex items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
          {meta.id !== 'trash' && <RoutineToggle meta={meta} />}
          {open && (
            <button className="btn btn-ghost btn-icon !w-8 !h-8" aria-label="Collapse" onClick={onToggle}>
              <ChevronDown size={18} style={{ transform: 'rotate(180deg)' }} />
            </button>
          )}
        </div>
      </motion.div>
      {!open && (
        <div className="relative px-5 pb-5 -mt-1 flex items-center gap-2">
          <div className="flex gap-1">
            {[...new Map(items.map((f) => [f.icon, f])).values()].slice(0, 4).map((f) => <Icon key={f.id} iconKey={f.icon} size={24} />)}
          </div>
          <span className="muted text-[14px] ml-1 truncate">{items.length === 1 ? items[0].title : `${items.length} items`}</span>
          {meta.id === 'trash' && <div className="ml-auto" onClick={(e) => e.stopPropagation()}><HoldButton className="!h-9 !px-4 !text-[13px]" onDone={() => remove(all)}><Trash2 size={15} /> Hold to empty</HoldButton></div>}
        </div>
      )}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }} className="relative">
            <div className="flex items-center gap-3 px-5 pb-3">
              <Checkbox checked={picked === all.length && all.length > 0} mixed={picked > 0 && picked < all.length} onChange={(v) => setMany(all, v)} color={meta.color} />
              <span className="muted text-[14px]">{picked ? `${picked} selected` : 'Select all'}</span>
              <div className="flex-1" />
              {meta.id === 'trash' ? (
                <HoldButton onDone={() => remove(all)}><Trash2 size={16} /> Hold to empty the Trash</HoldButton>
              ) : (
                all.some((i) => i.safety === 'safe') && (
                  <button className="btn" onClick={() => remove(all.filter((i) => i.safety === 'safe'))}>
                    <Sparkles size={16} /> Clean safe ones
                  </button>
                )
              )}
            </div>
            <div className="px-2 pb-3">
              <AnimatePresence initial={false}>
                {items.map((f) => <FindingRow key={f.id} f={f} color={meta.color} />)}
              </AnimatePresence>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
};

const FindingRow: React.FC<{ f: Finding; color: string }> = ({ f, color }) => {
  const picked = useStore((s) => s.selection.has(`f:${f.id}`));
  const toggle = useStore((s) => s.toggle);
  const remove = useStore((s) => s.remove);
  const item = sel.finding(f);
  const s = splitBytes(f.bytes);
  const row = (
    <div
      className="group flex items-center gap-4 px-3 py-3 rounded-2xl row-hover"
      style={{ opacity: f.busy ? 0.45 : 1, background: picked ? `${color}14` : undefined }}
      onClick={() => !f.busy && toggle(item)}
    >
      <Checkbox checked={picked} disabled={!!f.busy} onChange={() => toggle(item)} color={color} />
      <Icon iconKey={f.icon} size={40} />
      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="text-[16px] font-semibold truncate">{f.title}</span>
          {f.detail && <span className="muted text-[14px] truncate">{f.detail}</span>}
        </div>
        <div className="muted text-[13px] truncate mt-0.5">{f.busy ?? f.why}</div>
      </div>
      <div className="flex items-center gap-2.5 muted shrink-0">
        {f.action.type === 'command' && <Tip content={<>Disko asks <b>{f.action.label}</b> to clean it</>}><Terminal size={16} /></Tip>}
        {f.regrows && <Tip content="Comes back by itself"><Repeat size={16} /></Tip>}
        {f.admin && <Tip content="Needs your Mac password"><Lock size={16} /></Tip>}
        {f.safety === 'review' && f.category !== 'trash' && <Tip content="Goes to the Trash, so you can put it back"><Trash2 size={16} /></Tip>}
      </div>
      <span className="muted text-[13px] w-[86px] text-right shrink-0">{age(f.last_used)}</span>
      <div className="w-[92px] text-right shrink-0">
        <span className="num font-bold text-[18px]">{s.value}</span>
        <span className="num font-semibold text-[13px] muted ml-1">{s.unit}</span>
      </div>
      {f.category !== 'trash' ? (
        <button className={`btn btn-icon !w-9 !h-9 ${f.busy ? 'invisible' : 'opacity-0 group-hover:opacity-100'}`} aria-label="Remove" disabled={!!f.busy} onClick={(e) => { e.stopPropagation(); remove([item]); }}>
          <X size={17} />
        </button>
      ) : <span className="w-9" />}
    </div>
  );
  return (
    <motion.div layout initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, x: 60, height: 0, transition: { duration: 0.25 } }}>
      {f.busy ? <Tip content={f.busy} className="block">{row}</Tip> : row}
    </motion.div>
  );
};

const INTERVALS = [1, 2, 7, 14, 30];

/** A tiny clock: make this category a routine. Click the interval to change it. */
const RoutineToggle: React.FC<{ meta: CategoryMeta }> = ({ meta }) => {
  const memory = useStore((s) => s.memory);
  const routine = memory?.routines.find((r) => r.select.categories?.length === 1 && r.select.categories[0] === meta.id && !r.select.rules?.length);
  const on = !!routine?.enabled;
  const save = (r: Routine) => api.memoryUpdate({ type: 'set-routine', routine: r }).then((m) => useStore.setState({ memory: m }));
  const toggle = () => save(routine ? { ...routine, enabled: !routine.enabled } : {
    id: `routine-${meta.id}`, title: meta.label, select: { categories: [meta.id] }, every_days: meta.everyDays, min_age_days: meta.everyDays, enabled: true,
  });
  const cycle = () => {
    if (!routine) return;
    const next = INTERVALS[(INTERVALS.indexOf(routine.every_days) + 1) % INTERVALS.length] ?? 2;
    save({ ...routine, every_days: next, min_age_days: next });
  };
  return (
    <div className="flex flex-col items-end gap-1">
      <Tip content={on ? 'Cleaned automatically. Click to stop.' : 'Clean this automatically'}>
        <button
          aria-label="Routine"
          onClick={toggle}
          className="w-8 h-8 rounded-full flex items-center justify-center transition-all"
          style={{ background: on ? meta.color : 'var(--panel-2)', color: on ? 'white' : 'var(--muted)', boxShadow: on ? `0 6px 16px -6px ${meta.color}` : undefined }}
        >
          <Clock size={16} strokeWidth={2.2} />
        </button>
      </Tip>
      {on && (
        <Tip content="Change how often">
          <button onClick={cycle} className="num text-[12px] font-bold px-1.5 h-5 rounded-full row-hover whitespace-nowrap" style={{ color: meta.color }}>
            {routine!.every_days === 1 ? 'daily' : `${routine!.every_days}d`}
          </button>
        </Tip>
      )}
    </div>
  );
};

