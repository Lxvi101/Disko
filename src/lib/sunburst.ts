import { hierarchy, partition, type HierarchyRectangularNode } from 'd3-hierarchy';
import type { TreeNode, VolumeInfo } from '../types';

export const RINGS = [1, 1, 1, 1, 1, 0.16, 0.16, 0.16, 0.16];
export const START_ANGLE = Math.PI * 0.4;
export const BRANCH_HUES = [76, 193, 266, 302, 329, 338, 350];
export const HIDDEN_COLOR = '#81378d';

// Standard view is an overview. Recompute these thresholds after every drill-down,
// so a small folder becomes fully explorable when it becomes the current folder.
const STANDARD_LIMITS = [8, 12, 10, 8, 6, 4, 3, 2, 2];
const STANDARD_MIN_SHARE = [0, 0.025, 0.035, 0.05, 0.06, 0.09, 0.12, 0.18, 0.22];
const STANDARD_MIN_TOTAL = [0, 0.0025, 0.003, 0.004, 0.0045, 0.005, 0.006, 0.008, 0.01];

/** Hover previews use scan data, so chart grouping never hides folder contents. */
export function folderForHover(tree: TreeNode, path: string | null): TreeNode | null {
  if (!path) return null;
  const remainder = path.endsWith('/…');
  const target = remainder ? path.slice(0, -2) || '/' : path;
  const find = (n: TreeNode, parent: TreeNode | null): TreeNode | null => {
    if (n.path === target) {
      return remainder || (n.kind === 'directory' && (n.children.length > 0 || n.rest_total > 0)) ? n : parent;
    }
    for (const child of n.children) {
      const found = find(child, n);
      if (found) return found;
    }
    return null;
  };
  const folder = find(tree, null);
  return folder?.path === tree.path ? null : folder;
}

export function simplifyTree(tree: TreeNode): TreeNode {
  const normalize = (n: TreeNode): TreeNode => {
    const children = n.children.map(normalize);
    return { ...n, children, total: Math.max(n.total, n.rest_total + children.reduce((sum, c) => sum + c.total, 0)) };
  };
  const root = normalize(tree);
  const simplify = (n: TreeNode, depth: number): TreeNode => {
    if (depth >= STANDARD_LIMITS.length) return { ...n, children: [], rest_total: 0, rest_count: 0 };
    const minimum = Math.max(root.total * STANDARD_MIN_TOTAL[depth], n.total * STANDARD_MIN_SHARE[depth]);
    const ordered = [...n.children].sort((a, b) => b.total - a.total);
    const visible = ordered.filter((c, i) => i < STANDARD_LIMITS[depth] && c.total > 0 && c.total >= minimum);
    const visiblePaths = new Set(visible.map(c => c.path));
    const omitted = ordered.filter(c => !visiblePaths.has(c.path));
    return {
      ...n,
      children: visible.map(c => simplify(c, depth + 1)),
      rest_total: n.rest_total + omitted.reduce((sum, c) => sum + c.total, 0),
      rest_count: n.rest_count + omitted.length,
    };
  };
  return simplify(root, 0);
}

export interface Datum {
  name: string;
  path: string;
  kind: string;
  total: number;
  category: string;
  reason: string;
  isRest?: boolean;
  isHidden?: boolean;
  children?: Datum[];
}
export type ArcNode = HierarchyRectangularNode<Datum> & { hue: number };
export interface Layout {
  nodes: ArcNode[];
  byDepth: ArcNode[][];
  byPath: Map<string, ArcNode>;
  total: number;
  hidden: number;
}

function toDatum(n: TreeNode): Datum {
  const children = n.children.map(toDatum);
  if (n.rest_total > 0) children.push({
    name: 'smaller objects…', path: `${n.path.replace(/\/$/, '')}/…`, kind: 'rest',
    total: n.rest_total, category: 'other', reason: 'Items too small to draw individually.', isRest: true,
  });
  return { ...n, total: Math.max(n.total, children.reduce((sum, c) => sum + c.total, 0)), children };
}

/** Preserve each folder's own size, including bytes not represented by its children. */
export function layoutTree(tree: TreeNode, volume?: VolumeInfo): Layout {
  const datum = toDatum(tree);
  const scanned = datum.total;
  const hidden = volume ? Math.max(0, volume.used - scanned) : 0;
  if (hidden > 0) datum.children!.push({
    name: 'hidden space…', path: 'disko:hidden', kind: 'hidden', total: hidden,
    category: 'protected', reason: 'Used volume space not accounted for by the scan.', isHidden: true,
  });
  const total = scanned + hidden;
  // The root's own value reserves free space without creating a painted segment.
  datum.total = volume ? Math.max(volume.total, total + Math.max(0, volume.available)) : total;
  const root = hierarchy(datum).sum(d => Math.max(0, d.total - (d.children?.reduce((sum, c) => sum + c.total, 0) ?? 0)));
  root.sort((a, b) => Number(!!a.data.isHidden) - Number(!!b.data.isHidden)
    || Number(!!a.data.isRest) - Number(!!b.data.isRest) || (b.value ?? 0) - (a.value ?? 0));
  const part = partition<Datum>().size([2 * Math.PI, RINGS.length])(root);
  const nodes = part.descendants().filter(d => d.depth > 0 && d.depth <= RINGS.length && d.x1 - d.x0 > 0.0006) as ArcNode[];
  const byDepth: ArcNode[][] = Array.from({ length: RINGS.length + 1 }, () => []);
  const byPath = new Map<string, ArcNode>();
  for (const d of nodes) {
    let branch = d;
    while (branch.parent && branch.depth > 1) branch = branch.parent as ArcNode;
    const rank = part.children?.indexOf(branch) ?? 0;
    const spread = Math.min(140, (branch.x1 - branch.x0) * 48);
    const position = ((d.x0 + d.x1) / 2 - branch.x0) / (branch.x1 - branch.x0) - 0.5;
    d.hue = (BRANCH_HUES[rank % BRANCH_HUES.length] + position * spread + 360) % 360;
    byDepth[d.depth].push(d);
    byPath.set(d.data.path, d);
  }
  for (const arr of byDepth) arr.sort((a, b) => a.x0 - b.x0);
  return { nodes, byDepth, byPath, total, hidden };
}
