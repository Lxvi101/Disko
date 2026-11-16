import { describe, expect, test } from 'bun:test';
import { accountedDatum, isInside, layoutTree, withoutHidden } from '../src/lib/sunburst';
import { blockAt, layoutBlocks } from '../src/lib/treemap';
import type { TreeNode } from '../src/types';

const node = (path: string, total: number, children: TreeNode[] = [], rest = 0): TreeNode => ({
  path, name: path.split('/').pop() || 'Disk', total, children, kind: 'directory',
  category: 'review', reason: '', tag: '', action: 'quarantine', rest_total: rest, rest_count: rest ? 2 : 0,
});
const disk = () => node('/', 100, [
  node('/Users', 70, [node('/Users/me', 60, [node('/Users/me/Movies', 50), node('/Users/me/Code', 10)]), node('/Users/Shared', 10)]),
  node('/Library', 30),
]);

describe('hiding items from the map', () => {
  test('removes the item and shrinks every ancestor by its size', () => {
    const source = disk();
    const { tree, removed } = withoutHidden(source, new Set(['/Users/me/Movies']));
    expect(removed).toBe(50);
    expect(tree.total).toBe(50);
    expect(tree.children[0].total).toBe(20);
    expect(tree.children[0].children[0].children.map((c) => c.path)).toEqual(['/Users/me/Code']);
    expect(source.children[0].children[0].children).toHaveLength(2);
  });

  test('returns the same tree when nothing is hidden', () => {
    const source = disk();
    expect(withoutHidden(source, new Set()).tree).toBe(source);
    expect(withoutHidden(source, new Set(['/elsewhere'])).tree).toBe(source);
  });

  test('hidden bytes never reappear as unaccounted volume space', () => {
    const volume = { total: 200, available: 90, used: 110 };
    const { tree, removed } = withoutHidden(disk(), new Set(['/Library']));
    const layout = layoutTree(tree, volume, removed);
    expect(layout.hidden).toBe(10);
    expect(layout.total).toBe(80);
    expect(accountedDatum(tree, volume, removed).hidden).toBe(10);
  });

  test('isInside treats the volume root correctly', () => {
    expect(isInside('/Users', '/')).toBe(true);
    expect(isInside('/', '/')).toBe(false);
    expect(isInside('/Users/me', '/Users')).toBe(true);
    expect(isInside('/Usersx', '/Users')).toBe(false);
  });
});

describe('block layout', () => {
  test('areas follow sizes and folders contain their children', () => {
    const { datum } = accountedDatum(disk());
    const layout = layoutBlocks(datum, 400, 300);
    const users = layout.byPath.get('/Users')!;
    const library = layout.byPath.get('/Library')!;
    const area = (b: typeof users) => (b.x1 - b.x0) * (b.y1 - b.y0);
    expect(area(users) / area(library)).toBeCloseTo(70 / 30, 0);
    for (const b of layout.nodes) {
      const p = b.parent!;
      if (p.depth === 0) continue;
      expect(b.x0).toBeGreaterThanOrEqual(p.x0);
      expect(b.y0).toBeGreaterThanOrEqual(p.y0);
      expect(b.x1).toBeLessThanOrEqual(p.x1 + 1e-6);
      expect(b.y1).toBeLessThanOrEqual(p.y1 + 1e-6);
    }
    // Parents are painted before their contents.
    for (let i = 1; i < layout.nodes.length; i++) expect(layout.nodes[i].depth).toBeGreaterThanOrEqual(layout.nodes[i - 1].depth);
  });

  test('hit testing finds the deepest block', () => {
    const { datum } = accountedDatum(disk());
    const layout = layoutBlocks(datum, 400, 300);
    const movies = layout.byPath.get('/Users/me/Movies')!;
    expect(blockAt(layout, (movies.x0 + movies.x1) / 2, (movies.y0 + movies.y1) / 2)?.data.path).toBe('/Users/me/Movies');
    const users = layout.byPath.get('/Users')!;
    expect(blockAt(layout, users.x0 + 6, users.y0 + 4)?.data.path).toBe('/Users');
  });
});
