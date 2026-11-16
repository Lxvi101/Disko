import { describe, expect, test } from 'bun:test';
import { layoutTree, simplifyTree, folderForHover } from '../src/lib/sunburst';
import type { TreeNode } from '../src/types';

const node = (path: string, total: number, children: TreeNode[] = [], rest = 0): TreeNode => ({
  path, name: path.split('/').pop() || 'Disk', total, children, kind: 'directory',
  category: 'review', reason: '', tag: '', action: 'quarantine', rest_total: rest, rest_count: rest ? 2 : 0,
});

describe('standard overview detail', () => {
  test('groups small siblings without losing bytes or changing the source', () => {
    const children = Array.from({ length: 20 }, (_, i) => node(`/folder/${i}`, 20 - i));
    const original = node('/folder', 250, children, 10);
    const simplified = simplifyTree(original);
    expect(simplified.children).toHaveLength(8);
    expect(simplified.rest_total).toBe(10 + children.slice(8).reduce((sum, c) => sum + c.total, 0));
    expect(simplified.rest_count).toBe(14);
    expect(original.children).toHaveLength(20);
    expect(original.rest_total).toBe(10);
    expect(layoutTree(simplified).total).toBe(layoutTree(original).total);
  });
  test('drilling into a small branch makes its children visible again', () => {
    const branch = node('/root/small', 20, [node('/root/small/a', 10), node('/root/small/b', 5), node('/root/small/c', 5)]);
    const original = node('/root', 10000, [node('/root/big', 9980), branch]);
    const overview = simplifyTree(original);
    expect(overview.children.find(c => c.path === branch.path)?.children).toHaveLength(0);
    expect(simplifyTree(branch).children).toHaveLength(3);
  });
  test('caps outer detail and preserves the volume free-space wedge', () => {
    let chain = node('/root/leaf', 100);
    for (let i = 9; i >= 0; i--) chain = node(`/root/${i}`, 100, [chain]);
    const simplified = simplifyTree(chain);
    const full = layoutTree(chain, { total: 200, used: 100, available: 100 });
    const overview = layoutTree(simplified, { total: 200, used: 100, available: 100 });
    expect(Math.max(...overview.nodes.map(n => n.depth))).toBe(9);
    expect(overview.nodes.length).toBeLessThanOrEqual(full.nodes.length);
    expect(span(overview.byDepth[1][0])).toBeCloseTo(Math.PI);
    expect(overview.total).toBe(full.total);
  });
});
const span = (n: { x0: number; x1: number }) => n.x1 - n.x0;

describe('folder hover contents', () => {
  const file = { ...node('/home/Library/cache.db', 10), kind: 'file' };
  const library = node('/home/Library', 30, [file, node('/home/Library/Data', 20)]);
  const tree = node('/home', 40, [library, node('/home/Documents', 10)]);
  test('hovering a top-level folder previews its children', () => {
    expect(folderForHover(tree, library.path)).toBe(library);
    expect(folderForHover(tree, library.path)?.children).toEqual(library.children);
  });
  test('files and smaller-object groups preview their containing folder', () => {
    expect(folderForHover(tree, file.path)).toBe(library);
    expect(folderForHover(tree, '/home/Library/…')).toBe(library);
  });
  test('preview contents are available even when chart detail is grouped', () => {
    const largeTree = node('/home', 100000, [library, node('/home/Large', 99970)]);
    expect(simplifyTree(largeTree).children.find(c => c.path === library.path)?.children).toHaveLength(0);
    expect(folderForHover(largeTree, library.path)?.children).toHaveLength(2);
  });
  test('unknown paths and root remainders return the current folder view', () => {
    expect(folderForHover(tree, null)).toBeNull();
    expect(folderForHover(tree, '/missing')).toBeNull();
    expect(folderForHover(tree, '/home/…')).toBeNull();
  });
});

describe('sunburst space accounting', () => {
  test('reserves the exact free-space fraction and includes unscanned used space', () => {
    const layout = layoutTree(node('/', 60, [node('/Users', 40), node('/Apps', 20)]), { total: 100, used: 80, available: 20 });
    expect(layout.total).toBe(80);
    expect(layout.hidden).toBe(20);
    expect(layout.byDepth[1].reduce((sum, n) => sum + span(n), 0)).toBeCloseTo(2 * Math.PI * 0.8);
    expect(span(layout.byPath.get('disko:hidden')!)).toBeCloseTo(2 * Math.PI * 0.2);
    expect(layout.byDepth[1].at(-1)?.data.isHidden).toBe(true);
  });
  test('keeps local folder bytes and smaller objects without inflating children', () => {
    const layout = layoutTree(node('/folder', 100, [node('/folder/a', 60, [node('/folder/a/child', 30)])], 20));
    expect(span(layout.byPath.get('/folder/a')!)).toBeCloseTo(2 * Math.PI * 0.6);
    expect(span(layout.byPath.get('/folder/a/child')!)).toBeCloseTo(2 * Math.PI * 0.3);
    expect(span(layout.byPath.get('/folder/…')!)).toBeCloseTo(2 * Math.PI * 0.2);
    expect(layout.hidden).toBe(0);
  });
  test('handles stale parent totals from the leaves upward', () => {
    const layout = layoutTree(node('/', 10, [node('/a', 20, [node('/a/b', 50)])]));
    expect(layout.total).toBe(50);
    for (const n of layout.nodes) expect(span(n)).toBeCloseTo(2 * Math.PI);
  });
  test('never creates negative hidden space when scan totals exceed volume usage', () => {
    const layout = layoutTree(node('/', 120, [node('/a', 120)]), { total: 100, used: 80, available: 20 });
    expect(layout.hidden).toBe(0);
    expect(span(layout.byPath.get('/a')!)).toBeCloseTo(2 * Math.PI * 120 / 140);
  });
  test('empty disks and folders produce no painted arcs', () => {
    expect(layoutTree(node('/', 0), { total: 100, used: 0, available: 100 }).nodes).toEqual([]);
    expect(layoutTree(node('/empty', 0)).nodes).toEqual([]);
  });
});
