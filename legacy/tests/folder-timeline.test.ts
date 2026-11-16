import { describe, expect, test } from 'bun:test';
import { buildColumns, chainOf, periodEdges, placeColumns, timelinePeriods, MIN_SEGMENT } from '../src/lib/folderTimeline';
import { foldChains } from '../src/lib/treemap';
import { accountedDatum } from '../src/lib/sunburst';
import type { FolderTimeline, TreeNode } from '../src/types';

const node = (path: string, total: number, children: TreeNode[] = []): TreeNode => ({
  path, name: path.split('/').pop() || 'Disk', total, children, kind: 'directory',
  category: 'review', reason: '', tag: '', action: 'quarantine', rest_total: 0, rest_count: 0,
});

describe('timeline periods', () => {
  const now = new Date(2026, 9, 22, 15, 30);
  const periods = timelinePeriods(now, 2020);
  const labels = periods.map((p) => p.label);

  test('run from the oldest year to today, ascending and gap-free', () => {
    expect(labels.slice(-4)).toEqual(['Earlier in October', 'Past week', 'Yesterday', 'Today']);
    expect(labels).toContain('September');
    expect(labels).toContain('January 2025');
    expect(labels).toContain('2024');
    expect(labels[0]).toBe('Before 2020');
    for (let i = 1; i < periods.length; i++) expect(periods[i].start).toBeGreaterThan(periods[i - 1].start);
    expect(periodEdges(periods)).toHaveLength(periods.length - 1);
    expect(periods.at(-1)!.start).toBe(new Date(2026, 9, 22).getTime() / 1000);
  });

  test('early in a month, the past week swallows it', () => {
    const early = timelinePeriods(new Date(2026, 9, 3), 2020).map((p) => p.label);
    expect(early).not.toContain('Earlier in October');
    expect(early.slice(-4)).toEqual(['September', 'Past week', 'Yesterday', 'Today']);
  });
});

describe('timeline columns', () => {
  const data: FolderTimeline = {
    path: '/w',
    children: [
      { path: '/w/a', name: 'a', kind: 'directory', total: 1000, category: 'review', reason: '', newest: 1, bytes: [0, 900, 0, 100] },
      { path: '/w/b', name: 'b', kind: 'file', total: 11, category: 'review', reason: '', newest: 1, bytes: [10, 0, 0, 1] },
    ],
    rest: [0, 0, 0, 2],
    rest_count: 3,
    undated: 0,
  };
  const periods = [0, 1, 2, 3].map((i) => ({ start: i ? i * 100 : -Infinity, label: `P${i}`, tick: `p${i}` }));

  test('empty periods drop out and quiet neighbours merge to fit', () => {
    expect(buildColumns(data, periods, 10).map((c) => c.label)).toEqual(['P0', 'P1', 'P3']);
    const merged = buildColumns(data, periods, 2);
    expect(merged.map((c) => c.label)).toEqual(['P0 – P1', 'P3']);
    expect(merged[0].total).toBe(910);
  });

  test('stacks share one scale and thin entries join the cap', () => {
    const cols = placeColumns(buildColumns(data, periods, 10), 300, 200, data.children.length);
    const tall = cols[1];
    expect(tall.segments).toHaveLength(1);
    expect(tall.segments[0].h).toBeCloseTo(200);
    const recent = cols[2];
    // 100 of 103 bytes: tall enough to draw; the 1-byte file and the remainder fold into the cap.
    expect(recent.segments.map((s) => s.entry)).toEqual([0]);
    expect(recent.small).toEqual(expect.objectContaining({ bytes: 3, count: 1 }));
    expect(recent.segments[0].y + recent.segments[0].h).toBeCloseTo(200);
    // A column whose only content is under the threshold still shows its largest entry.
    expect(cols[0].segments.map((s) => s.entry)).toEqual([1]);
    expect(10 * (200 / 900)).toBeLessThan(MIN_SEGMENT);
  });
});

describe('single-child chains', () => {
  const tree = node('/w', 100, [
    node('/w/le', 100, [node('/w/le/outputs', 99, [node('/w/le/outputs/panel', 99, [node('/w/le/outputs/panel/a', 50), node('/w/le/outputs/panel/b', 49)])])]),
  ]);

  test('read as one path and open the innermost folder', () => {
    expect(chainOf(tree.children[0])).toEqual({ name: 'le / outputs / panel', path: '/w/le/outputs/panel' });
    expect(chainOf(tree.children[0].children[0].children[0])).toEqual({ name: 'panel', path: '/w/le/outputs/panel' });
  });

  test('fold into one block frame that keeps the outer path', () => {
    const folded = foldChains(accountedDatum(tree).datum);
    expect(folded.path).toBe('/w');
    const le = folded.children![0];
    expect(le).toEqual(expect.objectContaining({ path: '/w/le', name: 'le / outputs / panel', target: '/w/le/outputs/panel' }));
    expect(le.children!.map((c) => c.name)).toEqual(['a', 'b']);
  });
});
