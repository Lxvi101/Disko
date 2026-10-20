import React, { useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Check, ChevronDown, Copy, FolderOpen, LockKeyhole, Minus, Terminal } from 'lucide-react';
import { useStore } from '../store';
import { api } from '../lib/api';
import { formatBytes } from '../lib/format';
import type { Cluster, ClusterGroup, ClusterItem } from '../lib/programs';

const selectable = (i: ClusterItem) => !i.blocked && !i.command;
const SHOW = 8;

/** A round check that can also be "some". */
export const Tick: React.FC<{ state: boolean | 'some'; onClick?: () => void; disabled?: boolean; color?: string; label: string; size?: number }> = ({ state, onClick, disabled, color, label, size = 20 }) => (
  <button
    type="button"
    role="checkbox"
    aria-checked={state === 'some' ? 'mixed' : state}
    aria-label={label}
    disabled={disabled}
    className="tick"
    data-on={state !== false}
    style={{ width: size, height: size, '--tick-color': color ?? 'var(--accent)' } as React.CSSProperties}
    onClick={(e) => { e.preventDefault(); e.stopPropagation(); onClick?.(); }}
  >
    <AnimatePresence initial={false}>
      {state !== false && (
        <motion.span key={String(state)} initial={{ scale: 0.3, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={{ scale: 0.3, opacity: 0 }} transition={{ type: 'spring', stiffness: 700, damping: 30 }} className="flex">
          {state === 'some' ? <Minus className="w-3 h-3" strokeWidth={3.5} /> : <Check className="w-3 h-3" strokeWidth={3.5} />}
        </motion.span>
      )}
    </AnimatePresence>
  </button>
);

function stateOf(items: ClusterItem[], selected: Set<string>): boolean | 'some' {
  const avail = items.filter(selectable);
  const n = avail.filter((i) => selected.has(i.id)).length;
  return n === 0 ? false : n === avail.length ? true : 'some';
}

export function selectedBytes(clusters: Cluster[], selected: Set<string>) {
  let bytes = 0, count = 0;
  for (const c of clusters) for (const g of c.groups) for (const i of g.items) if (selected.has(i.id)) { bytes += i.total; count++; }
  return { bytes, count };
}

export const ClusterBrowser: React.FC<{
  clusters: Cluster[];
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  busy?: boolean;
}> = ({ clusters, selected, onChange, busy }) => {
  const [picked, setPicked] = useState<string | null>(null);
  // Until the user picks a program, keep showing the biggest one, even as results stream in and reorder.
  const active = picked && clusters.some((c) => c.program.id === picked) ? picked : clusters[0]?.program.id ?? null;
  const setActive = setPicked;
  const current = clusters.find((c) => c.program.id === active) ?? clusters[0];
  const grand = clusters.reduce((n, c) => n + c.total, 0);

  const setMany = (items: ClusterItem[], on: boolean) => {
    const next = new Set(selected);
    for (const i of items.filter(selectable)) on ? next.add(i.id) : next.delete(i.id);
    onChange(next);
  };
  const flip = (items: ClusterItem[]) => setMany(items, stateOf(items, selected) !== true);

  return (
    <div className="cluster-browser">
      <aside className="cluster-list" aria-label="Programs">
        <div className="cluster-share" aria-hidden="true">
          {clusters.map((c) => <span key={c.program.id} style={{ flexGrow: Math.max(c.total, grand * 0.01), background: c.program.color }} title={`${c.program.name} · ${formatBytes(c.total)}`} />)}
        </div>
        {clusters.map((c, idx) => {
          const items = c.groups.flatMap((g) => g.items);
          const st = stateOf(items, selected);
          const sel = items.filter((i) => selected.has(i.id)).reduce((n, i) => n + i.total, 0);
          return (
            <motion.div
              role="button"
              tabIndex={0}
              onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setActive(c.program.id); } }}
              layout
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: Math.min(idx * 0.03, 0.4) }}
              key={c.program.id}
              className="cluster-card"
              aria-current={c.program.id === current?.program.id}
              onClick={() => setActive(c.program.id)}
              style={{ '--p-color': c.program.color } as React.CSSProperties}
            >
              <span className="program-tile">{c.program.emoji}</span>
              <span className="flex-1 min-w-0 text-left">
                <span className="block text-[13px] font-semibold truncate">{c.program.name}</span>
                <span className="block text-[11px] truncate" style={{ color: 'var(--dim)' }}>
                  {sel > 0 ? <><span style={{ color: 'var(--text)' }}>{formatBytes(sel)}</span> selected</> : `${c.count} ${c.count === 1 ? 'item' : 'items'}`}
                </span>
              </span>
              <span className="tnum text-[13px] font-semibold">{c.total === 0 ? <span className="size-pending" /> : formatBytes(c.total)}</span>
              <Tick state={st} disabled={busy || !items.some(selectable)} color={c.program.color} label={`Select all of ${c.program.name}`} onClick={() => flip(items)} />
            </motion.div>
          );
        })}
      </aside>

      <section className="cluster-detail" aria-label={current?.program.name}>
        <AnimatePresence mode="wait" initial={false}>
          {current && (
            <motion.div key={current.program.id} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={{ duration: 0.16 }}>
              <header className="flex items-center gap-4 mb-5" style={{ '--p-color': current.program.color } as React.CSSProperties}>
                <span className="program-tile large">{current.program.emoji}</span>
                <div className="flex-1 min-w-0">
                  <h2 className="text-[22px] font-bold tracking-tight leading-tight">{current.program.name}</h2>
                  <p className="text-[12.5px]" style={{ color: 'var(--muted)' }}>{current.program.blurb}</p>
                </div>
                <span className="tnum text-[24px] font-bold tracking-tight">{current.total === 0 ? <span className="size-pending" /> : formatBytes(current.total)}</span>
              </header>
              {current.groups.map((g) => <GroupSection key={g.id} group={g} color={current.program.color} selected={selected} busy={busy} onToggle={(items) => flip(items)} />)}
            </motion.div>
          )}
        </AnimatePresence>
      </section>
    </div>
  );
};

function GroupSection({ group, color, selected, busy, onToggle }: { group: ClusterGroup; color: string; selected: Set<string>; busy?: boolean; onToggle: (items: ClusterItem[]) => void }) {
  const [all, setAll] = useState(false);
  const [open, setOpen] = useState(true);
  const toast = useStore((s) => s.toast);
  const shown = all ? group.items : group.items.slice(0, SHOW);
  const st = stateOf(group.items, selected);
  const single = group.items.length === 1;
  const copy = (cmd: string) => navigator.clipboard?.writeText(cmd).then(() => toast({ kind: 'success', title: 'Command copied', detail: cmd })).catch(() => {});
  const hidden = useMemo(() => group.items.length - shown.length, [group.items.length, shown.length]);

  return (
    <div className="cluster-group" style={{ '--p-color': color } as React.CSSProperties}>
      <div className="cluster-group-head" onClick={() => !single && setOpen(!open)} role={single ? undefined : 'button'}>
        <Tick state={st} disabled={busy || !group.items.some(selectable)} color={color} label={`Select ${group.title}`} onClick={() => onToggle(group.items)} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-semibold">{group.title}</span>
            {group.method === 'command' && <span className="cluster-badge" title={group.command ?? ''}><Terminal className="w-3 h-3" /> uses the tool’s own cleanup</span>}
          </div>
          <p className="text-[11.5px] mt-0.5" style={{ color: 'var(--muted)' }}>{[group.blurb, group.after].filter(Boolean).join(' ')}</p>
        </div>
        <span className="tnum text-[14px] font-semibold">{group.items.some((i) => i.measured === false) && group.total === 0 ? <span className="size-pending" /> : formatBytes(group.total)}</span>
        {!single && <ChevronDown className="w-4 h-4 transition-transform" style={{ color: 'var(--dim)', transform: open ? 'none' : 'rotate(-90deg)' }} />}
      </div>
      {open && !single && (
        <div className="cluster-items">
          {shown.map((i) => (
            <div key={i.id} className="cluster-item group" data-off={!selectable(i)}>
              {i.blocked ? <LockKeyhole className="w-3.5 h-3.5 m-[3px]" style={{ color: 'var(--dim)' }} /> : i.command ? <Terminal className="w-3.5 h-3.5 m-[3px]" style={{ color: 'var(--dim)' }} /> :
                <Tick size={18} state={selected.has(i.id)} disabled={busy} color={color} label={`Select ${i.label}`} onClick={() => onToggle([i])} />}
              <div className="flex-1 min-w-0">
                <div className="text-[12.5px] font-medium truncate">{i.label}</div>
                <div className="text-[11px] truncate selectable" style={{ color: i.blocked ? 'var(--warn)' : 'var(--dim)' }} title={i.path}>{i.blocked ?? i.detail}</div>
              </div>
              {i.command && (
                <button className="junk-command" title="Copy command. This tool knows which files are still in use." onClick={() => copy(i.command!)}>
                  <span className="truncate">{i.command}</span><Copy className="w-3 h-3 flex-shrink-0 opacity-60" />
                </button>
              )}
              <span className="tnum text-[12.5px] font-semibold w-[70px] text-right">{i.measured === false ? <span className="size-pending" /> : formatBytes(i.total)}</span>
              {i.path && <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Show in Finder" onClick={() => api.reveal(i.path).catch(() => {})}><FolderOpen className="w-3.5 h-3.5" /></button>}
            </div>
          ))}
          {hidden > 0 && <button className="cluster-more" onClick={() => setAll(true)}>Show {hidden} more</button>}
          {all && group.items.length > SHOW && <button className="cluster-more" onClick={() => setAll(false)}>Show fewer</button>}
        </div>
      )}
    </div>
  );
}
