import React, { useEffect, useMemo, useState } from 'react';
import { ChevronLeft, ChevronRight, Copy, FileText, FolderOpen, Hammer, Package, RefreshCw, Search, Sparkles, Terminal, Trash2, Archive, Database } from 'lucide-react';
import { useStore } from '../../store';
import { api, errorText } from '../../lib/api';
import { formatBytes, shortPath } from '../../lib/format';
import { JUNK_GROUPS, describeJunk, junkGroup, type JunkGroup } from '../../lib/junk';
import { Empty, SegmentedControl, Spinner } from '../ui';
import type { JunkItem, StagedItem } from '../../types';

const PAGE = 50;
const ICONS: Record<JunkGroup, React.FC<{ className?: string }>> = { deps: Package, build: Hammer, cache: Database, logs: FileText, installers: Archive };

/** "3 mo old", "2 yr old": how long since the folder last changed. */
export function age(secs: number | null): string {
  if (!secs) return '';
  const days = (Date.now() / 1000 - secs) / 86400;
  if (days < 1) return 'today';
  if (days < 30) return `${Math.floor(days)} d old`;
  if (days < 365) return `${Math.floor(days / 30)} mo old`;
  return `${Math.floor(days / 365)} yr old`;
}

const toStaged = (it: JunkItem): StagedItem => ({ path: it.path, name: it.name, kind: it.kind, total: it.total, category: it.category, reason: it.reason, tag: it.tag, action: it.action, tool: it.tool, source: 'suggestion' });

export const JunkPage: React.FC = () => {
  const home = useStore((s) => s.info?.home ?? '');
  const version = useStore((s) => s.dataVersion);
  const db = useStore((s) => s.info?.db_path);
  const busy = useStore((s) => s.cleanupBusy);
  const toast = useStore((s) => s.toast);
  const [items, setItems] = useState<JunkItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [group, setGroup] = useState<JunkGroup | null>(null);
  const [q, setQ] = useState('');
  const [sort, setSort] = useState<'size' | 'age'>('size');
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => {
    let live = true;
    setError(null);
    api.junk().then((r) => { if (live) { setItems(r); setSelected((old) => new Set([...old].filter((p) => r.some((i) => i.path === p)))); } }).catch((e) => live && setError(errorText(e)));
    return () => { live = false; };
  }, [version, db, refresh]);

  const totals = useMemo(() => {
    const t = Object.fromEntries(JUNK_GROUPS.map((g) => [g.id, { bytes: 0, count: 0 }])) as Record<JunkGroup, { bytes: number; count: number }>;
    for (const it of items ?? []) { const g = t[junkGroup(it).id]; g.bytes += it.total; g.count += 1; }
    return t;
  }, [items]);
  const rows = useMemo(() => {
    const s = q.trim().toLowerCase();
    const r = (items ?? []).filter((it) => (!group || junkGroup(it).id === group) && (!s || it.path.toLowerCase().includes(s)));
    return sort === 'age' ? [...r].sort((a, b) => (a.mtime ?? Infinity) - (b.mtime ?? Infinity)) : r;
  }, [items, group, q, sort]);
  useEffect(() => setPage(0), [group, q, sort]);

  const pages = Math.max(1, Math.ceil(rows.length / PAGE));
  const current = Math.min(page, pages - 1);
  const shown = rows.slice(current * PAGE, current * PAGE + PAGE);
  const cleanable = (it: JunkItem) => it.action === 'quarantine';
  const shownCleanable = shown.filter(cleanable);
  const pageAll = shownCleanable.length > 0 && shownCleanable.every((i) => selected.has(i.path));
  const chosen = (items ?? []).filter((i) => selected.has(i.path));
  const chosenBytes = chosen.reduce((n, i) => n + i.total, 0);
  const total = (items ?? []).reduce((n, i) => n + i.total, 0);
  const toggle = (path: string) => setSelected((old) => { const next = new Set(old); next.has(path) ? next.delete(path) : next.add(path); return next; });
  const clean = () => {
    if (!chosen.length || useStore.getState().cleanupBusy) return;
    useStore.setState({ cleanupSelection: chosen.map(toStaged), cleanupMode: 'quarantine', cleanModalOpen: true });
  };

  if (error) return <Empty icon={<Trash2 className="w-6 h-6" />} title="Could not look for junk" body={error} action={<button className="btn" onClick={() => setRefresh((n) => n + 1)}>Try again</button>} />;
  if (!items) return <div className="h-full flex items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--muted)' }}><Spinner /> Looking for junk…</div>;
  if (!items.length) return <Empty icon={<Sparkles className="w-6 h-6" />} title="No junk found" body="No dependencies, build files, caches, logs or installers over 1 MB in this scan." />;

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[1120px] mx-auto px-8 pt-7 pb-6">
        <header className="flex items-end justify-between gap-4">
          <div>
            <h1 className="text-[26px] font-bold tracking-tight leading-none">Junk</h1>
            <p className="text-[13px] mt-2" style={{ color: 'var(--muted)' }}>
              <strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(total)}</strong> in {items.length} places that apps and tools can recreate.
            </p>
          </div>
          <button className="btn btn-ghost btn-icon" title="Look again" onClick={() => { setItems(null); setRefresh((n) => n + 1); }}><RefreshCw className="w-4 h-4" /></button>
        </header>

        <div className="junk-groups mt-6" role="group" aria-label="Kinds of junk">
          {JUNK_GROUPS.filter((g) => totals[g.id].count).map((g) => {
            const Icon = ICONS[g.id];
            return (
              <button key={g.id} className="junk-group" aria-pressed={group === g.id} title={g.blurb} onClick={() => setGroup(group === g.id ? null : g.id)} style={{ '--junk-color': g.color } as React.CSSProperties}>
                <span className="junk-icon"><Icon className="w-4 h-4" /></span>
                <span className="block text-[12.5px] font-medium mt-3">{g.label}</span>
                <span className="block tnum text-[19px] font-semibold tracking-tight mt-0.5">{formatBytes(totals[g.id].bytes)}</span>
                <span className="block text-[11px] mt-0.5" style={{ color: 'var(--dim)' }}>{totals[g.id].count} {totals[g.id].count === 1 ? 'place' : 'places'}</span>
              </button>
            );
          })}
        </div>
        {group && <p className="text-[12px] mt-3" style={{ color: 'var(--muted)' }}>{JUNK_GROUPS.find((g) => g.id === group)!.blurb} <button className="underline underline-offset-2" onClick={() => setGroup(null)}>Show everything</button></p>}

        <div className="flex items-center gap-3 mt-6 mb-2">
          <label className="relative w-64">
            <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--dim)' }} />
            <input className="input !pl-8 !py-1.5 !text-[12.5px]" placeholder="Filter by project or path" value={q} onChange={(e) => setQ(e.target.value)} />
          </label>
          <SegmentedControl value={sort} onChange={setSort} options={[{ value: 'size', label: 'Largest' }, { value: 'age', label: 'Oldest', title: 'Least recently changed first' }]} />
          <span className="flex-1" />
          <label className="flex items-center gap-2 text-[12px]" style={{ color: 'var(--muted)' }}>
            <input type="checkbox" checked={pageAll} disabled={!shownCleanable.length || busy} onChange={(e) => setSelected((old) => { const next = new Set(old); shownCleanable.forEach((i) => e.target.checked ? next.add(i.path) : next.delete(i.path)); return next; })} />
            Select this page
          </label>
        </div>

        <div className="junk-list">
          {shown.map((it) => {
            const g = junkGroup(it);
            const Icon = ICONS[g.id];
            const d = describeJunk(it);
            const manual = !cleanable(it);
            return (
              <label key={it.path} className="junk-row group" aria-disabled={manual} onContextMenu={(e) => e.preventDefault()}>
                <input type="checkbox" aria-label={`Select ${d.title}`} disabled={manual || busy} checked={selected.has(it.path)} onChange={() => toggle(it.path)} />
                <span className="junk-icon small" style={{ '--junk-color': g.color } as React.CSSProperties}><Icon className="w-3.5 h-3.5" /></span>
                <span className="flex-1 min-w-0">
                  <span className="flex items-center gap-2 min-w-0">
                    <span className="text-[13px] font-medium truncate">{d.title}</span>
                    <span className="junk-kind">{d.kind}</span>
                    {it.category === 'review' && <span className="junk-kind warn" title={it.reason}>Check first</span>}
                  </span>
                  <span className="block text-[11.5px] truncate mt-0.5 selectable" style={{ color: 'var(--dim)' }} title={`${it.path}\n\n${it.reason}`}>{shortPath(it.path, home)}</span>
                </span>
                {manual && it.tool ? (
                  <button className="junk-command" title="Copy command. This tool knows which files are still in use." onClick={(e) => { e.preventDefault(); navigator.clipboard?.writeText(it.tool!).then(() => toast({ kind: 'success', title: 'Command copied', detail: it.tool! })).catch(() => {}); }}>
                    <Terminal className="w-3 h-3 flex-shrink-0" /><span className="truncate">{it.tool}</span><Copy className="w-3 h-3 flex-shrink-0 opacity-60" />
                  </button>
                ) : (
                  <span className="text-[11.5px] w-[76px] text-right flex-shrink-0" style={{ color: 'var(--dim)' }}>{age(it.mtime)}</span>
                )}
                <span className="tnum text-[13px] font-semibold w-[72px] text-right flex-shrink-0">{formatBytes(it.total)}</span>
                <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Show in Finder" onClick={(e) => { e.preventDefault(); api.reveal(it.path).catch(() => {}); }}><FolderOpen className="w-3.5 h-3.5" /></button>
              </label>
            );
          })}
          {!shown.length && <p className="py-12 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>Nothing matches “{q}”.</p>}
        </div>

        {pages > 1 && (
          <nav className="flex items-center justify-between mt-3 text-[12px]" aria-label="Pages" style={{ color: 'var(--muted)' }}>
            <span className="tnum">{current * PAGE + 1}–{Math.min(rows.length, (current + 1) * PAGE)} of {rows.length}</span>
            <span className="flex items-center gap-1">
              <button className="btn btn-ghost btn-icon" disabled={current === 0} onClick={() => setPage(current - 1)} aria-label="Previous page"><ChevronLeft className="w-4 h-4" /></button>
              {pageNumbers(current, pages).map((n, i) => n < 0
                ? <span key={`gap${i}`} className="px-1">…</span>
                : <button key={n} className="junk-page tnum" aria-current={n === current ? 'page' : undefined} onClick={() => setPage(n)}>{n + 1}</button>)}
              <button className="btn btn-ghost btn-icon" disabled={current >= pages - 1} onClick={() => setPage(current + 1)} aria-label="Next page"><ChevronRight className="w-4 h-4" /></button>
            </span>
          </nav>
        )}

        {chosen.length > 0 && (
          <div className="action-bar">
            <span className="text-[12.5px] flex-1" style={{ color: 'var(--muted)' }}>
              <strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(chosenBytes)}</strong> selected from {chosen.length} {chosen.length === 1 ? 'place' : 'places'}
            </span>
            <button className="btn btn-ghost" disabled={busy} onClick={() => setSelected(new Set())}>Clear</button>
            <button className="btn btn-primary" disabled={busy} onClick={clean}>Clean up…</button>
          </div>
        )}
      </div>
    </div>
  );
};

/** Page indexes to show, with -1 for a gap: 1 … 4 5 6 … 12 */
function pageNumbers(current: number, pages: number): number[] {
  const out: number[] = [];
  for (let i = 0; i < pages; i++) {
    if (i === 0 || i === pages - 1 || Math.abs(i - current) <= 1) out.push(i);
    else if (out[out.length - 1] !== -1) out.push(-1);
  }
  return out;
}
