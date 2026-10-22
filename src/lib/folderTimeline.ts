import type { FolderTimeline, TreeNode } from '../types';
import { hsl } from './colors';

/** A stretch of time on the folder timeline. Periods are ascending and touch end to start. */
export interface Period {
  /** Seconds since the epoch; the oldest period starts at -Infinity. */
  start: number;
  /** Full name for tooltips: "Today", "September", "March 2025", "2023". */
  label: string;
  /** Axis label: "Today", "Sep", "’23". */
  tick: string;
}

const DAY = 86400;
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const secs = (d: Date) => Math.floor(d.getTime() / 1000);

/**
 * Human periods in local time: days for the last week, months for this year and last,
 * whole years before that, back to `firstYear`. A period ends where the next one starts,
 * so a month that overlaps "Past week" simply loses those days to it.
 */
export function timelinePeriods(now: Date, firstYear = 1990): Period[] {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const y = now.getFullYear();
  const newestFirst: Period[] = [
    { start: secs(today), label: 'Today', tick: 'Today' },
    { start: secs(today) - DAY, label: 'Yesterday', tick: 'Yest.' },
    { start: secs(today) - 6 * DAY, label: 'Past week', tick: 'Week' },
  ];
  for (let m = now.getMonth(); m >= -12; m--) {
    const d = new Date(y, m, 1);
    const sameYear = d.getFullYear() === y;
    const name = MONTHS[d.getMonth()];
    newestFirst.push({
      start: secs(d),
      label: m === now.getMonth() ? `Earlier in ${name}` : sameYear ? name : `${name} ${d.getFullYear()}`,
      tick: sameYear ? name.slice(0, 3) : `${name.slice(0, 3)} ’${String(d.getFullYear()).slice(2)}`,
    });
  }
  for (let year = y - 2; year >= firstYear; year--) {
    newestFirst.push({ start: secs(new Date(year, 0, 1)), label: String(year), tick: String(year) });
  }
  newestFirst.push({ start: -Infinity, label: `Before ${firstYear}`, tick: `<${firstYear}` });
  // Drop any period the newer ones already cover (early in a month, "Past week" swallows it).
  const out: Period[] = [];
  for (const p of newestFirst) if (!out.length || p.start < out[out.length - 1].start) out.push(p);
  return out.reverse();
}

/** What `get_folder_timeline` needs: every period start except the open-ended oldest. */
export const periodEdges = (periods: Period[]) => periods.slice(1).map((p) => p.start);

export interface Entry {
  key: string;
  /** Path the entry stands for; clicking navigates here (the end of a collapsed chain). */
  path: string;
  /** "le / outputs / panel" when single-child folders were folded into one. */
  name: string;
  kind: string;
  total: number;
  category: string;
  reason: string;
  newest: number | null;
  color: string;
  isRest?: boolean;
  /** For the remainder: how many items it stands for. */
  count?: number;
}

export interface Column {
  key: string;
  label: string;
  tick: string;
  total: number;
  /** Bytes per entry index. */
  bytes: Map<number, number>;
}

/**
 * Columns for the periods that hold anything, oldest first. When there are more than
 * `max`, the adjacent pair holding the least is merged until they fit: quiet years fold
 * into "2014 – 2019" while busy months keep their own column.
 */
export function buildColumns(data: FolderTimeline, periods: Period[], max: number): Column[] {
  const restIndex = data.children.length;
  let cols: Column[] = [];
  periods.forEach((p, i) => {
    const bytes = new Map<number, number>();
    data.children.forEach((c, k) => { if (c.bytes[i] > 0) bytes.set(k, c.bytes[i]); });
    if (data.rest[i] > 0) bytes.set(restIndex, data.rest[i]);
    const total = [...bytes.values()].reduce((a, b) => a + b, 0);
    if (total > 0) cols.push({ key: String(p.start), label: p.label, tick: p.tick, total, bytes });
  });
  const fit = Math.max(1, max);
  while (cols.length > fit) {
    let best = 0;
    for (let i = 1; i < cols.length - 1; i++) if (cols[i].total + cols[i + 1].total < cols[best].total + cols[best + 1].total) best = i;
    const a = cols[best], b = cols[best + 1];
    const bytes = new Map(a.bytes);
    for (const [k, v] of b.bytes) bytes.set(k, (bytes.get(k) ?? 0) + v);
    const from = a.label.split(' – ')[0], to = b.label.split(' – ').pop()!;
    const fromTick = a.tick.split('–')[0], toTick = b.tick.split('–').pop()!;
    cols = [...cols.slice(0, best), { key: a.key, label: `${from} – ${to}`, tick: `${fromTick}–${toTick}`, total: a.total + b.total, bytes }, ...cols.slice(best + 2)];
  }
  return cols;
}

export interface Segment {
  entry: number;
  bytes: number;
  y: number;
  h: number;
}
export interface PlacedColumn extends Column {
  x: number;
  w: number;
  segments: Segment[];
  /** Bytes of entries too thin to draw, shown as one "N smaller items" cap. */
  small: { bytes: number; count: number; y: number; h: number } | null;
}

/** Smallest segment drawn on its own; thinner ones join the column's "smaller items" cap. */
export const MIN_SEGMENT = 5;

/**
 * Stack each column bottom-up, largest entry at the base, with height on one linear scale
 * shared by all columns so a tall column really holds more. `restIndex` always goes in the cap.
 */
export function placeColumns(cols: Column[], width: number, height: number, restIndex: number, gap = 8, maxW = 120): PlacedColumn[] {
  if (!cols.length) return [];
  const w = Math.max(8, Math.min(maxW, (width - gap * (cols.length - 1)) / cols.length));
  const left = (width - (w * cols.length + gap * (cols.length - 1))) / 2;
  const peak = Math.max(...cols.map((c) => c.total));
  const scale = height / peak;
  return cols.map((c, i) => {
    const colH = Math.max(3, c.total * scale);
    const ordered = [...c.bytes].filter(([k]) => k !== restIndex).sort((a, b) => b[1] - a[1]);
    const segments: Segment[] = [];
    let smallBytes = c.bytes.get(restIndex) ?? 0, smallCount = 0;
    for (const [entry, bytes] of ordered) {
      if (bytes * scale >= MIN_SEGMENT) segments.push({ entry, bytes, y: 0, h: 0 });
      else { smallBytes += bytes; smallCount++; }
    }
    if (smallBytes && !segments.length) {
      // A quiet column still shows its largest item rather than a lone grey stub.
      const [entry, bytes] = ordered[0] ?? [];
      if (entry !== undefined) { segments.push({ entry, bytes, y: 0, h: 0 }); smallBytes -= bytes; smallCount--; }
    }
    let y = height;
    for (const s of segments) {
      s.h = (s.bytes / c.total) * colH;
      y -= s.h;
      s.y = y;
    }
    const small = smallBytes > 0 ? { bytes: smallBytes, count: smallCount, h: (smallBytes / c.total) * colH, y: 0 } : null;
    if (small) small.y = y - small.h;
    return { ...c, x: left + i * (w + gap), w, segments, small };
  });
}

/**
 * Follow a folder through children that are its only content, so "outputs" holding only
 * "open-motion" holding only "panel" reads as one entry that opens the innermost folder.
 */
export function chainOf(node: TreeNode): { name: string; path: string } {
  const names = [node.name];
  let n = node;
  while (n.kind === 'directory' && n.children.length === 1 && n.rest_total === 0 && n.children[0].kind === 'directory' && n.children[0].total >= n.total * 0.98) {
    n = n.children[0];
    names.push(n.name);
  }
  return { name: names.join(' / '), path: n.path };
}

// Calm, evenly spaced hues that stay apart from each other and from the stale/hover greys.
const HUES = [212, 162, 262, 28, 330, 190, 96, 292, 8, 52];

/** Entry colour: soft, readable in both themes; files stay neutral so folders lead. */
export function entryColor(index: number, kind: string, light: boolean): string {
  if (kind !== 'directory') return light ? hsl(220, 0.1, 0.72) : hsl(220, 0.08, 0.42);
  const hue = HUES[index % HUES.length];
  const round = Math.floor(index / HUES.length);
  return light ? hsl(hue, 0.52, 0.66 - round * 0.06) : hsl(hue, 0.42, 0.56 - round * 0.06);
}
