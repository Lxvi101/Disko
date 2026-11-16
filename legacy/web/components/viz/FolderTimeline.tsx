import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useStore } from '../../store';
import { api, errorText } from '../../lib/api';
import { formatBytes, relativeTime, shortPath } from '../../lib/format';
import { inkFor, restColor } from '../../lib/colors';
import { buildColumns, chainOf, entryColor, periodEdges, placeColumns, timelinePeriods, type Entry, type PlacedColumn } from '../../lib/folderTimeline';
import type { FolderTimeline as Data, TreeNode } from '../../types';
import type { MenuTarget } from '../ContextMenu';
import { ZoomStage, useHeldLayout } from './Stage';
import { Spinner } from '../ui';

interface Props {
  path: string;
  /** Current folder's tree, used to fold single-child chains into one entry. */
  tree: TreeNode | null;
  /** Hidden items inside this folder; their bytes leave the timeline too. */
  hidden: string[];
  hoverPath: string | null;
  stagedPaths: Set<string>;
  onHover: (path: string | null) => void;
  onNavigate: (path: string) => void;
  onRemainder: (path: string) => void;
  onLayout: (colors: Map<string, string>) => void;
  onContextMenu?: (d: MenuTarget, e: React.MouseEvent) => void;
}

const TOP = 40;
const AXIS = 34;
const HEADROOM = 22;
const STAGED = 'hsl(0 0% 60%)';

const cache = new Map<string, Data>();

interface View {
  entries: Entry[];
  cols: PlacedColumn[];
  restIndex: number;
  plotH: number;
  width: number;
  height: number;
  recent: number;
  stale: number;
  undated: number;
}

/** "in September", "today", "between 2014 and 2019": how a column reads inside a sentence. */
function when(label: string): string {
  const one = (l: string) => (l === 'Today' || l === 'Yesterday' ? l.toLowerCase() : l === 'Past week' ? 'the past week' : l.replace(/^Earlier in /, ''));
  if (label.includes(' – ')) {
    const [a, b] = label.split(' – ');
    return `between ${one(a)} and ${one(b)}`;
  }
  if (label === 'Today' || label === 'Yesterday') return label.toLowerCase();
  if (label === 'Past week') return 'in the past week';
  return label.startsWith('Earlier in ') ? `earlier in ${label.slice(11)}` : `in ${label}`;
}

export const FolderTimeline: React.FC<Props> = ({ path, tree, hidden, hoverPath, stagedPaths, onHover, onNavigate, onRemainder, onLayout, onContextMenu }) => {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });
  const light = useStore((s) => s.resolvedTheme === 'light');
  const home = useStore((s) => s.info?.home ?? '');
  const source = `${useStore((s) => s.info?.db_path)}:${useStore((s) => s.dataVersion)}`;
  const day = new Date().toDateString();
  const periods = useMemo(() => timelinePeriods(new Date()), [day]); // eslint-disable-line react-hooks/exhaustive-deps
  const hiddenKey = hidden.join('\n');
  const key = `${source}:${day}:${path}:${hiddenKey}`;
  const [data, setData] = useState<{ key: string; data: Data } | null>(() => (cache.has(key) ? { key, data: cache.get(key)! } : null));
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [tip, setTip] = useState<{ x: number; y: number; title: string; lines: string[]; sub?: string } | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setBox({ width: Math.floor(entry.contentRect.width), height: Math.floor(entry.contentRect.height) }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const hit = cache.get(key);
    if (hit) { setData({ key, data: hit }); setError(null); return; }
    let live = true;
    setLoading(true);
    setError(null);
    api.folderTimeline(path, periodEdges(periods), hidden)
      .then((d) => {
        if (cache.size > 40) cache.delete(cache.keys().next().value!);
        cache.set(key, d);
        if (live) setData({ key, data: d });
      })
      .catch((e) => live && setError(errorText(e)))
      .finally(() => live && setLoading(false));
    return () => { live = false; };
    // `hidden` is tracked through `key`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, periods]);

  const current = data?.data.path === path ? data.data : null;

  const view: View | null = useMemo(() => {
    if (!current || box.width < 40 || box.height < 120) return null;
    const byPath = new Map((tree?.path === path ? tree.children : []).map((c) => [c.path, c]));
    const entries: Entry[] = current.children.map((c, i) => {
      const node = byPath.get(c.path);
      const chain = node ? chainOf(node) : { name: c.name, path: c.path };
      return { key: c.path, path: chain.path, name: chain.name, kind: c.kind, total: c.total, category: c.category, reason: c.reason, newest: c.newest, color: entryColor(i, c.kind, light) };
    });
    const restIndex = entries.length;
    entries.push({
      key: `${path.replace(/\/$/, '')}/…`, path, name: 'smaller items', kind: 'rest', total: current.rest.reduce((a, b) => a + b, 0),
      category: 'other', reason: '', newest: null, color: restColor(light), isRest: true, count: current.rest_count,
    });
    const plotH = box.height - TOP - AXIS - HEADROOM;
    const cols = placeColumns(buildColumns(current, periods, Math.floor((box.width + 8) / 52)), box.width, plotH, restIndex);
    const now = Date.now() / 1000;
    let recent = 0, stale = 0;
    periods.forEach((p, i) => {
      const end = periods[i + 1]?.start ?? Infinity;
      const bytes = current.children.reduce((n, c) => n + c.bytes[i], current.rest[i]);
      if (p.start >= now - 31 * 86400) recent += bytes;
      if (end <= now - 365 * 86400) stale += bytes;
    });
    return { entries, cols, restIndex, plotH, width: box.width, height: box.height, recent, stale, undated: current.undated };
  }, [current, tree, path, box, periods, light]);

  const held = useHeldLayout(view, view ? path : null);
  const shown = held?.layout ?? null;

  useEffect(() => {
    if (!shown) return;
    onLayout(new Map(shown.entries.map((e) => [e.key, e.color])));
  }, [shown, onLayout]);

  const active = shown ? shown.entries.findIndex((e) => e.key === hoverPath || (hoverPath != null && hoverPath.startsWith(e.key + '/') && !e.isRest)) : -1;
  const isStaged = (p: string) => stagedPaths.has(p) || [...stagedPaths].some((s) => p.startsWith(s + '/'));

  const leave = () => { setTip(null); onHover(null); };
  const point = (ev: React.MouseEvent) => {
    const r = wrapRef.current!.getBoundingClientRect();
    return { x: ev.clientX - r.left, y: ev.clientY - r.top };
  };

  const segment = (v: View, col: PlacedColumn, entry: number, bytes: number, y: number, h: number, small?: number) => {
    const e = v.entries[entry];
    const rest = small !== undefined || e.isRest;
    const color = rest ? restColor(light) : isStaged(e.key) ? STAGED : e.color;
    const dim = active >= 0 && active !== entry;
    const name = rest ? `${small ?? e.count} smaller ${(small ?? e.count) === 1 ? 'item' : 'items'}` : e.name;
    const roomy = col.w >= 56;
    return (
      <div
        key={rest ? 'small' : e.key}
        className="ftl-seg"
        draggable={!rest}
        style={{ top: y, height: Math.max(1, h), background: color, color: inkFor(color), opacity: dim ? 0.32 : 1 }}
        onMouseMove={(ev) => {
          const p = point(ev);
          if (!rest && hoverPath !== e.key) onHover(e.key);
          if (rest && hoverPath) onHover(null);
          setTip({
            ...p,
            title: name,
            lines: [`${formatBytes(bytes)} last changed ${when(col.label)}`, ...(!rest ? [`${formatBytes(e.total)} in all${e.newest ? ` · newest change ${relativeTime(e.newest)}` : ''}`] : [])],
            sub: rest ? undefined : shortPath(e.path, home),
          });
        }}
        onClick={() => {
          if (rest) return onRemainder(path);
          if (e.kind === 'directory') onNavigate(e.path);
        }}
        onContextMenu={(ev) => {
          if (rest) return;
          ev.preventDefault();
          onContextMenu?.({ path: e.key, name: e.name.split(' / ')[0], kind: e.kind, total: e.total, category: e.category, reason: e.reason }, ev);
        }}
        onDragStart={(ev) => {
          // Same payload as list rows, the wheel and the blocks, so the collector accepts it unchanged.
          ev.dataTransfer.setData('application/x-disko-item', JSON.stringify({ path: e.key, name: e.name.split(' / ')[0], kind: e.kind, total: e.total, category: e.category, reason: e.reason }));
          ev.dataTransfer.effectAllowed = 'copy';
        }}
        onMouseLeave={() => { setTip(null); onHover(null); }}
        onDragEnd={leave}
      >
        {roomy && h >= 20 && (
          <span className="ftl-seg-label">
            <span className="truncate font-medium">{name}</span>
            {h >= 36 && <span className="tnum opacity-70">{formatBytes(bytes)}</span>}
          </span>
        )}
      </div>
    );
  };

  return (
    <div ref={wrapRef} className="relative w-full h-full" onMouseLeave={leave}>
      {held && shown && (
        <ZoomStage id={held.path} direction={held.direction} width={shown.width} height={shown.height}>
          <div className="ftl" style={{ width: shown.width, height: shown.height }}>
            <div className="ftl-summary">
              {shown.cols.length ? (
                <>
                  <span><strong className="tnum">{formatBytes(shown.recent)}</strong> changed in the last month</span>
                  <span className="ftl-summary-sep" />
                  <span><strong className="tnum">{formatBytes(shown.stale)}</strong> untouched for over a year</span>
                </>
              ) : <span>Nothing here has a modification date.</span>}
            </div>
            <div className="ftl-plot" style={{ top: TOP + HEADROOM, height: shown.plotH }}>
              <div className="ftl-baseline" style={{ top: shown.plotH }} />
              {shown.cols.map((c) => {
                const stackTop = Math.min(...c.segments.map((s) => s.y), c.small?.y ?? Infinity);
                return (
                  <div key={c.key} className="ftl-col" style={{ left: c.x, width: c.w, height: shown.plotH }}>
                    <span className="ftl-total tnum" style={{ top: stackTop - 18 }}>{c.w >= 44 ? formatBytes(c.total, 1) : ''}</span>
                    <div className="ftl-stack" style={{ top: stackTop, height: shown.plotH - stackTop }}>
                      {c.small && segment(shown, c, shown.restIndex, c.small.bytes, 0, c.small.h, c.small.count + (c.bytes.has(shown.restIndex) ? shown.entries[shown.restIndex].count ?? 0 : 0))}
                      {c.segments.map((s) => segment(shown, c, s.entry, s.bytes, s.y - stackTop, s.h))}
                    </div>
                    <span className="ftl-tick" title={c.label} style={{ top: shown.plotH + 8 }}>{c.tick}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </ZoomStage>
      )}
      {tip && (
        <div className="tl-tip" style={{ left: Math.max(0, Math.min(tip.x + 14, box.width - 250)), top: tip.y - 12, marginTop: 0, transform: 'translateY(-100%)' }}>
          <div className="text-[12.5px] font-semibold break-words">{tip.title}</div>
          {tip.lines.map((l, i) => <div key={i} className="text-[11.5px] mt-0.5" style={{ color: i ? 'var(--muted)' : 'var(--text)' }}>{l}</div>)}
          {tip.sub && <div className="text-[11px] mt-1 truncate" style={{ color: 'var(--dim)' }}>{tip.sub}</div>}
        </div>
      )}
      {error && !shown && <div className="absolute inset-0 flex items-center justify-center text-[12.5px] px-8 text-center" style={{ color: 'var(--muted)' }}>Could not build this folder’s timeline. {error}</div>}
      {(loading || (!shown && !error)) && (
        <div className="absolute top-2 right-2 pointer-events-none" style={{ zIndex: 2 }}>
          <Spinner size={14} />
        </div>
      )}
    </div>
  );
};
