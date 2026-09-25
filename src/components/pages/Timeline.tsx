import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Copy, FolderOpen, History, RefreshCw, Terminal } from 'lucide-react';
import { useStore } from '../../store';
import { api, errorText } from '../../lib/api';
import { formatBytes, shortPath } from '../../lib/format';
import { Empty, SegmentedControl, Spinner } from '../ui';
import { memberToStaged } from './Suggestions';
import type { OldItem, StagedItem, TimelineLane } from '../../types';

const DAY = 86400;
const LANES: { id: TimelineLane; label: string }[] = [
  { id: 'app', label: 'Apps' },
  { id: 'project', label: 'Projects' },
  { id: 'file', label: 'Large files' },
  { id: 'download', label: 'Downloads & Desktop' },
  { id: 'developer', label: 'Developer' },
  { id: 'backup', label: 'Backups & leftovers' },
];
const THRESHOLDS = [{ value: '90', label: '3 months' }, { value: '182', label: '6 months' }, { value: '365', label: '1 year' }, { value: '730', label: '2 years' }] as const;
const TICKS: [number, string][] = [[0, 'Today'], [7, '1 wk'], [30, '1 mo'], [90, '3 mo'], [182, '6 mo'], [365, '1 yr'], [730, '2 yr'], [1825, '5 yr'], [3650, '10 yr']];
const BUCKETS: [number, string][] = [[1825, 'Over 5 years ago'], [730, '2 to 5 years ago'], [365, '1 to 2 years ago'], [182, '6 to 12 months ago'], [90, '3 to 6 months ago'], [30, '1 to 3 months ago']];

/** A timeline entry plus what cleaning it would actually remove. */
type Entry = OldItem & { key: string; age: number | null; staged: StagedItem[]; note: string };

const laneColor = (lane: TimelineLane) => `var(--lane-${lane})`;
const month = (secs: number) => new Date(secs * 1000).toLocaleDateString(undefined, { month: 'short', year: 'numeric' });
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

function toEntry(it: OldItem, now: number): Entry {
  const age = it.date ? Math.max(0, (now - it.date) / DAY) : null;
  const one = (): StagedItem[] => [{ path: it.path, name: it.name, kind: it.lane === 'file' || it.label === 'Installer' ? 'file' : 'directory', total: it.total, category: it.category, reason: `${cap(it.date_kind)} ${it.date ? month(it.date) : 'never, as far as macOS knows'}.`, action: it.action, source: 'inactive' }];
  let staged: StagedItem[] = [];
  let note = it.detail;
  if (it.lane === 'project') {
    // An old project keeps its source; only its generated folders are offered.
    staged = it.junk.map(([path, total]) => ({ path, name: path.split('/').pop() ?? path, kind: 'directory', total, category: 'rebuildable', reason: `Generated folder in ${it.name}, a project last worked on ${it.date ? month(it.date) : 'long ago'}.`, source: 'inactive' }));
    const bytes = it.junk.reduce((n, [, b]) => n + b, 0);
    note = bytes ? `${formatBytes(bytes)} of dependencies and build files can go; the source stays` : 'No build files to clear. Archive it yourself if you are done with it.';
  } else if (it.category !== 'protected' && it.action === 'quarantine') {
    staged = one();
  } else if (it.lane === 'backup' && it.action !== 'quarantine') {
    note = 'Remove in Finder: select the device, then Manage Backups';
  }
  return { ...it, key: it.path, age, staged, note };
}

let cache: { key: string; items: OldItem[] } | null = null;

export const TimelinePage: React.FC = () => {
  const home = useStore((s) => s.info?.home ?? '');
  const key = `${useStore((s) => s.info?.db_path)}:${useStore((s) => s.dataVersion)}`;
  const remnants = useStore((s) => s.remnants);
  const busy = useStore((s) => s.cleanupBusy);
  const toast = useStore((s) => s.toast);
  const [items, setItems] = useState<OldItem[] | null>(cache?.key === key ? cache.items : null);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [threshold, setThreshold] = useState<string>('365');
  const [lane, setLane] = useState<TimelineLane | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);

  useEffect(() => {
    if (cache?.key === key && !refresh) return;
    let live = true;
    setError(null);
    api.timeline().then((r) => { if (live) { cache = { key, items: r }; setItems(r); } }).catch((e) => live && setError(errorText(e)));
    return () => { live = false; };
  }, [key, refresh]);

  const now = Date.now() / 1000;
  const entries: Entry[] = useMemo(() => {
    const out = (items ?? []).map((i) => toEntry(i, now));
    // Leftovers of uninstalled apps are old by definition; place them by their last change.
    for (const g of remnants?.groups ?? []) {
      const staged = g.members.filter((m) => !m.persistent).map((m) => memberToStaged(g, m));
      out.push({
        path: g.members[0]?.path ?? g.id, key: `remnant:${g.id}`, name: g.name, lane: 'backup', label: 'Leftover of removed app', total: g.total,
        date: g.newest_mtime ?? null, date_kind: 'last changed', detail: '', category: 'review', action: 'quarantine', junk: [],
        age: g.newest_mtime ? Math.max(0, (now - g.newest_mtime) / DAY) : null, staged,
        note: staged.length ? `${g.id} is no longer installed; its caches can go` : `${g.id} is no longer installed; only settings remain`,
      });
    }
    return out;
    // `now` moves every render; the list only needs to follow the data.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, remnants]);

  const min = Number(threshold);
  const stale = entries.filter((e) => e.age !== null && e.age >= min);
  const staleBytes = stale.reduce((n, e) => n + e.total, 0);
  const listed = entries.filter((e) => (!lane || e.lane === lane) && (e.age === null || e.age >= min));
  // Every cutoff is also a bucket edge, so each listed item falls into exactly one bucket.
  const edges = BUCKETS.filter(([b]) => b >= min);
  const buckets = [
    ...edges.map(([b, label]) => ({ id: label, label, items: listed.filter((e) => e.age !== null && edges.find(([x]) => e.age! >= x)?.[0] === b).sort((a, c) => c.total - a.total) })),
    { id: 'none', label: 'No usage record', items: listed.filter((e) => e.age === null).sort((a, c) => c.total - a.total) },
  ].filter((b) => b.items.length);

  const byKey = useMemo(() => new Map(entries.map((e) => [e.key, e])), [entries]);
  const chosen = [...selected].map((k) => byKey.get(k)).filter((e): e is Entry => !!e && e.staged.length > 0);
  const chosenBytes = chosen.reduce((n, e) => n + e.staged.reduce((m, s) => m + s.total, 0), 0);
  const toggle = (k: string) => setSelected((old) => { const n = new Set(old); n.has(k) ? n.delete(k) : n.add(k); return n; });
  const clean = () => {
    if (!chosen.length || useStore.getState().cleanupBusy) return;
    useStore.setState({ cleanupSelection: chosen.flatMap((e) => e.staged), cleanupMode: 'quarantine', cleanModalOpen: true });
  };
  const reveal = (e: Entry) => {
    const target = byKey.get(e.key);
    if (!target) return;
    if (lane && lane !== target.lane) setLane(null);
    const bucket = buckets.find((b) => b.items.some((i) => i.key === target.key));
    if (bucket) setExpanded((old) => new Set(old).add(bucket.id));
    setFocus(target.key);
  };
  useLayoutEffect(() => {
    if (!focus) return;
    document.getElementById(`tl-${focus}`)?.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    const t = setTimeout(() => setFocus(null), 1600);
    return () => clearTimeout(t);
  }, [focus]);

  if (error) return <Empty icon={<History className="w-6 h-6" />} title="Could not build the timeline" body={error} action={<button className="btn" onClick={() => setRefresh((n) => n + 1)}>Try again</button>} />;
  if (!items) return <div className="h-full flex flex-col items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--muted)' }}><Spinner /> Checking when you last used your apps, projects and files…</div>;

  const thresholdLabel = THRESHOLDS.find((t) => t.value === threshold)!.label;
  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[1120px] mx-auto px-8 pt-7 pb-8">
        <header className="flex items-end justify-between gap-4">
          <div>
            <h1 className="text-[26px] font-bold tracking-tight leading-none">Timeline</h1>
            <p className="text-[13px] mt-2" style={{ color: 'var(--muted)' }}>
              <strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(staleBytes)}</strong> in {stale.length} things you haven’t used in over {thresholdLabel}.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-[12px]" style={{ color: 'var(--muted)' }}>Unused for</span>
            <SegmentedControl value={threshold} onChange={setThreshold} options={THRESHOLDS.map((t) => ({ value: t.value, label: t.label }))} />
            <button className="btn btn-ghost btn-icon" title="Check again" onClick={() => { setItems(null); setRefresh((n) => n + 1); }}><RefreshCw className="w-4 h-4" /></button>
          </div>
        </header>

        <TimelineChart entries={entries} threshold={min} lane={lane} onLane={(l) => setLane(lane === l ? null : l)} onPick={reveal} />

        <div className="mt-8">
          {lane && (
            <p className="text-[12.5px] mb-3" style={{ color: 'var(--muted)' }}>
              Showing {LANES.find((l) => l.id === lane)!.label.toLowerCase()} only. <button className="underline underline-offset-2" onClick={() => setLane(null)}>Show everything</button>
            </p>
          )}
          {buckets.map((b) => {
            const open = expanded.has(b.id);
            const shown = open ? b.items : b.items.slice(0, 12);
            return (
              <section key={b.id} className="tl-bucket">
                <div className="tl-bucket-head">
                  <span className="tl-rail-dot" />
                  <h2 className="text-[15px] font-semibold">{b.label}</h2>
                  <span className="tnum text-[12.5px]" style={{ color: 'var(--muted)' }}>{formatBytes(b.items.reduce((n, e) => n + e.total, 0))} · {b.items.length}</span>
                </div>
                <div className="junk-list">
                  {shown.map((e) => {
                    const canClean = e.staged.length > 0;
                    const command = !canClean && e.tool && e.action === 'manager_command';
                    return (
                      <label key={e.key} id={`tl-${e.key}`} className="junk-row group" aria-disabled={!canClean} data-focus={focus === e.key || undefined}>
                        <input type="checkbox" disabled={!canClean || busy} checked={selected.has(e.key)} onChange={() => toggle(e.key)} aria-label={`Select ${e.name}`} />
                        <span className="w-2.5 h-2.5 rounded-full flex-shrink-0" style={{ background: laneColor(e.lane) }} />
                        <span className="flex-1 min-w-0">
                          <span className="flex items-center gap-2 min-w-0">
                            <span className="text-[13px] font-medium truncate">{e.name}</span>
                            <span className="junk-kind">{e.label}</span>
                          </span>
                          <span className="block text-[11.5px] truncate mt-0.5" style={{ color: 'var(--dim)' }} title={e.path}>{e.note || shortPath(e.path, home)}</span>
                        </span>
                        {command && (
                          <button className="junk-command" title="Copy command" onClick={(ev) => { ev.preventDefault(); navigator.clipboard?.writeText(e.tool!).then(() => toast({ kind: 'success', title: 'Command copied', detail: e.tool! })).catch(() => {}); }}>
                            <Terminal className="w-3 h-3 flex-shrink-0" /><span className="truncate">{e.tool}</span><Copy className="w-3 h-3 flex-shrink-0 opacity-60" />
                          </button>
                        )}
                        <span className="text-[11.5px] w-[124px] text-right flex-shrink-0" style={{ color: 'var(--dim)' }}>{e.date ? `${cap(e.date_kind)} ${month(e.date)}` : 'No record'}</span>
                        <span className="tnum text-[13px] font-semibold w-[72px] text-right flex-shrink-0">{formatBytes(e.total)}</span>
                        <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Show in Finder" onClick={(ev) => { ev.preventDefault(); api.reveal(e.path).catch(() => {}); }}><FolderOpen className="w-3.5 h-3.5" /></button>
                      </label>
                    );
                  })}
                </div>
                {b.items.length > 12 && (
                  <button className="btn btn-ghost btn-sm mt-2" onClick={() => setExpanded((old) => { const n = new Set(old); open ? n.delete(b.id) : n.add(b.id); return n; })}>
                    {open ? 'Show fewer' : `Show all ${b.items.length}`}
                  </button>
                )}
              </section>
            );
          })}
          {!buckets.length && <p className="py-12 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>Nothing unused for over {thresholdLabel}.</p>}
          <p className="text-[11px] mt-6 max-w-[760px] leading-relaxed" style={{ color: 'var(--dim)' }}>
            Dates come from Spotlight’s last-opened record, git activity, simulator boots and file changes. “Last seen” means an app’s own files changed then. Age is a hint, not proof: check before you remove anything.
          </p>
        </div>

        {chosen.length > 0 && (
          <div className="action-bar">
            <span className="text-[12.5px] flex-1" style={{ color: 'var(--muted)' }}><strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(chosenBytes)}</strong> selected from {chosen.length} {chosen.length === 1 ? 'item' : 'items'}</span>
            <button className="btn btn-ghost" disabled={busy} onClick={() => setSelected(new Set())}>Clear</button>
            <button className="btn btn-primary" disabled={busy} onClick={clean}>Clean up…</button>
          </div>
        )}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Chart: one lane per kind, x = time since last use on a log scale, area = size.
// ---------------------------------------------------------------------------

const GUTTER = 190;
const NO_RECORD = 64;
const LANE_H = 62;
const AXIS_H = 30;

type Dot = { e: Entry; x: number; y: number; r: number };

function TimelineChart({ entries, threshold, lane, onLane, onPick }: { entries: Entry[]; threshold: number; lane: TimelineLane | null; onLane: (l: TimelineLane) => void; onPick: (e: Entry) => void }) {
  const wrap = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1000);
  const [hover, setHover] = useState<Dot | null>(null);
  useLayoutEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(560, e.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const lanes = LANES.filter((l) => entries.some((e) => e.lane === l.id));
  const oldest = Math.max(3 * 365, ...entries.map((e) => e.age ?? 0));
  const maxAge = Math.min(oldest, 15 * 365);
  const plotL = GUTTER + NO_RECORD;
  const plotW = width - plotL - 16;
  const x = (age: number) => plotL + plotW * (1 - Math.log1p(Math.min(age, maxAge)) / Math.log1p(maxAge));
  const height = lanes.length * LANE_H + AXIS_H;

  const dots = useMemo(() => {
    const maxBytes = Math.max(1, ...entries.map((e) => e.total));
    const out: Dot[] = [];
    lanes.forEach((l, li) => {
      const cy = li * LANE_H + LANE_H / 2;
      const placed: Dot[] = [];
      // Largest first, then nudge each dot up or down until it clears its neighbours (a small beeswarm).
      for (const e of entries.filter((i) => i.lane === l.id).sort((a, b) => b.total - a.total)) {
        const r = 4 + 14 * Math.sqrt(e.total / maxBytes);
        const dx = e.age === null ? GUTTER + NO_RECORD / 2 : x(e.age);
        const room = LANE_H / 2 - r - 3;
        let y = cy;
        for (let k = 0; k <= 16; k++) {
          const off = k === 0 ? 0 : (k % 2 ? 1 : -1) * Math.ceil(k / 2) * 3;
          if (Math.abs(off) > room) break;
          y = cy + off;
          if (placed.every((p) => Math.hypot(p.x - dx, p.y - y) >= p.r + r + 1)) break;
        }
        const d = { e, x: dx, y: Math.max(cy - Math.max(room, 0), Math.min(cy + Math.max(room, 0), y)), r };
        placed.push(d);
        out.push(d);
      }
    });
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entries, width, lanes.length]);

  const pick = (ev: React.MouseEvent<SVGSVGElement>) => {
    const box = ev.currentTarget.getBoundingClientRect();
    const px = ev.clientX - box.left, py = ev.clientY - box.top;
    let best: Dot | null = null, bestD = Infinity;
    for (const d of dots) {
      const dist = Math.hypot(d.x - px, d.y - py) - d.r;
      if (dist < 6 && dist < bestD) { best = d; bestD = dist; }
    }
    return best;
  };
  const tx = x(threshold);

  return (
    <div ref={wrap} className="tl-chart relative mt-6">
      <svg width={width} height={height} role="img" aria-label="Timeline of when items were last used, by kind and size"
        onPointerMove={(ev) => setHover(pick(ev))} onPointerLeave={() => setHover(null)} onClick={(ev) => { const d = pick(ev); if (d) onPick(d.e); }}
        style={{ cursor: hover ? 'pointer' : 'default', display: 'block' }}>
        {/* the stale region: everything older than the chosen cutoff */}
        <rect x={plotL} y={0} width={Math.max(0, tx - plotL)} height={lanes.length * LANE_H} fill="var(--tl-stale)" rx={6} />
        <line x1={tx} x2={tx} y1={0} y2={lanes.length * LANE_H} stroke="var(--muted)" strokeDasharray="3 3" />
        <line x1={GUTTER + NO_RECORD - 4} x2={GUTTER + NO_RECORD - 4} y1={4} y2={lanes.length * LANE_H - 4} stroke="var(--line-2)" strokeDasharray="2 3" />
        {lanes.map((l, i) => (
          <g key={l.id}>
            {i > 0 && <line x1={GUTTER} x2={width - 16} y1={i * LANE_H} y2={i * LANE_H} stroke="var(--line)" />}
          </g>
        ))}
        {TICKS.filter(([d]) => d <= maxAge).map(([d, label]) => (
          <g key={d}>
            <line x1={x(d)} x2={x(d)} y1={lanes.length * LANE_H} y2={lanes.length * LANE_H + 4} stroke="var(--line-2)" />
            <text x={x(d)} y={lanes.length * LANE_H + 18} textAnchor={d === 0 ? 'end' : 'middle'} className="tl-tick">{d === 0 ? label : `${label} ago`}</text>
          </g>
        ))}
        <text x={GUTTER + NO_RECORD / 2 - 4} y={lanes.length * LANE_H + 18} textAnchor="middle" className="tl-tick">No record</text>
        {dots.map((d) => {
          const recent = d.e.age !== null && d.e.age < threshold;
          const dim = (lane && d.e.lane !== lane) || recent;
          return <circle key={d.e.key} cx={d.x} cy={d.y} r={d.r} fill={laneColor(d.e.lane)} fillOpacity={dim ? 0.22 : 0.9} stroke="var(--bg-2)" strokeWidth={hover?.e.key === d.e.key ? 0 : 2} />;
        })}
        {hover && <circle cx={hover.x} cy={hover.y} r={hover.r + 3} fill="none" stroke="var(--text)" strokeWidth={1.5} pointerEvents="none" />}
      </svg>

      {/* lane labels double as the legend and as filters */}
      {lanes.map((l, i) => {
        const inLane = entries.filter((e) => e.lane === l.id && e.age !== null && e.age >= threshold);
        return (
          <button key={l.id} className="tl-lane" aria-pressed={lane === l.id} style={{ top: i * LANE_H, height: LANE_H, width: GUTTER - 12 }} onClick={() => onLane(l.id)}>
            <span className="w-2.5 h-2.5 rounded-full flex-shrink-0" style={{ background: laneColor(l.id) }} />
            <span className="min-w-0 text-left">
              <span className="block text-[12.5px] font-medium truncate">{l.label}</span>
              <span className="block text-[11px] tnum" style={{ color: 'var(--dim)' }}>{inLane.length ? `${formatBytes(inLane.reduce((n, e) => n + e.total, 0))} unused` : 'All recent'}</span>
            </span>
          </button>
        );
      })}

      {hover && (
        <div className="tl-tip" style={{ left: Math.min(hover.x + 14, width - 260), top: Math.max(0, hover.y - 12) }}>
          <div className="text-[12.5px] font-semibold truncate">{hover.e.name}</div>
          <div className="text-[11.5px]" style={{ color: 'var(--muted)' }}>{hover.e.label} · <span className="tnum">{formatBytes(hover.e.total)}</span></div>
          <div className="text-[11.5px] mt-1">
            {hover.e.date ? <>{cap(hover.e.date_kind)} {month(hover.e.date)} <span style={{ color: 'var(--dim)' }}>· {ageText(hover.e.age!)}</span></> : 'macOS has no record of use'}
          </div>
        </div>
      )}
    </div>
  );
}

function ageText(days: number) {
  if (days < 1) return 'today';
  if (days < 30) return `${Math.round(days)} days ago`;
  if (days < 365) return `${Math.round(days / 30)} months ago`;
  const y = days / 365;
  return `${y < 2 ? y.toFixed(1) : Math.floor(y)} years ago`;
}
