import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { catOf } from '../../lib/categories';
import { applyGone, useStore } from '../../store';
import type { TreeNode, VolumeInfo } from '../../lib/types';

/** A tree node plus what the map needs to draw it. */
export type MapNode = TreeNode & { hue?: number; why?: string };

// Ordinary folders stay in one quiet slate family around a cool axis; only clutter carries colour.
const BRANCH_HUES = [222, 210, 234, 200, 244, 216, 228, 194];

/** Calm slate for ordinary folders, the category tone for clutter. */
export function colorOf(n: TreeNode, hue: number, depth: number, dark: boolean): string {
  const cat = catOf(n.clutter);
  if (cat) return cat.color;
  if (n.kind === 'hidden') return dark ? '#3d3c4a' : '#d4d2de';
  if (n.kind === 'other') return dark ? '#24272f' : '#dfe1e7';
  const file = n.kind === 'file';
  const s = file ? 14 : dark ? 30 : 34;
  const l = dark ? 28 + depth * 4.5 + (file ? -3 : 0) : 84 - depth * 3.5 + (file ? 4 : 0);
  return `hsl(${hue} ${s}% ${l}%)`;
}

export function hueFor(index: number) {
  return BRANCH_HUES[index % BRANCH_HUES.length];
}

/** At the disk root, space no folder shows becomes its own explained slices. */
export function withHidden(tree: TreeNode, volume: VolumeInfo | null): MapNode {
  if (tree.path !== '/' || !volume?.hidden.length || !tree.children) return tree;
  const hidden: MapNode[] = volume.hidden.filter((h) => h.bytes > 0).map((h) => ({
    name: h.label, path: `hidden:${h.kind}`, kind: 'hidden', bytes: h.bytes, files: 0, why: h.why,
  }));
  const children = [...tree.children, ...hidden].sort((a, b) => rank(a) - rank(b) || b.bytes - a.bytes);
  return { ...tree, bytes: tree.bytes + hidden.reduce((a, h) => a + h.bytes, 0), children };
}
const rank = (n: TreeNode) => (n.kind === 'hidden' ? 2 : n.kind === 'other' ? 1 : 0);

/** A folder whose only content is one folder shows as one item ("Users › alex"); opening it goes straight in. */
export function merge(n: MapNode): MapNode {
  let cur = n;
  const names = [n.name];
  while (cur.children?.length === 1 && cur.children[0].kind === 'dir' && cur.children[0].bytes >= cur.bytes * 0.98) {
    cur = cur.children[0];
    names.push(cur.name);
  }
  const children = cur.children?.map(merge);
  return cur === n ? (children ? { ...n, children } : n) : { ...cur, name: names.join(' › '), children };
}

export const isNavigable = (n: TreeNode) => n.kind === 'dir';

/** Depth each view needs below the current folder. */
export const DEPTH = { sunburst: 5, bubbles: 2, blocks: 3, list: 1 } as const;

/**
 * Fetches the current folder at the depth the view needs, keeps it fresh while the scan streams
 * (at most ~1/s) and after removals, and applies optimistic removals on top.
 */
export function useSpaceTree(): { tree: MapNode | null; loading: boolean } {
  const path = useStore((s) => s.spacePath);
  const view = useStore((s) => s.spaceView);
  const version = useStore((s) => s.treeVersion);
  const scanTick = useStore((s) => s.scanTick);
  const gone = useStore((s) => s.gone);
  const volume = useStore((s) => s.volume);
  const setSpaceTree = useStore((s) => s.setSpaceTree);
  const [raw, setRaw] = useState<TreeNode | null>(null);
  const [loading, setLoading] = useState(true);
  const last = useRef(0);
  const timer = useRef(0);
  const depth = DEPTH[view];

  // Navigation and view changes fetch right away.
  useEffect(() => {
    let live = true;
    setLoading(true);
    api.tree(path === '/' ? null : path, depth).then((t) => {
      if (!live) return;
      // A folder that vanished (removed elsewhere) sends us to its parent.
      setRaw(t);
      setLoading(false);
      last.current = performance.now();
    }).catch(() => live && setLoading(false));
    return () => { live = false; };
  }, [path, depth]);

  // Removals, undo, outside changes and the streaming scan: refetch at most ~1/s, and keep the
  // current tree when nothing visible changed so views don't re-layout or re-animate.
  const accept = (t: TreeNode) => setRaw((old) => (old && similar(old, t) ? old : t));
  useEffect(() => {
    if (!version && !scanTick) return;
    const run = () => {
      last.current = performance.now();
      api.tree(path === '/' ? null : path, depth).then(accept).catch(() => {});
    };
    const since = performance.now() - last.current;
    clearTimeout(timer.current);
    if (since > 1000) run();
    else timer.current = window.setTimeout(run, 1000 - since);
    return () => clearTimeout(timer.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version, scanTick]);

  // While the next folder loads, the previous one stays up so views can morph between them.
  // Only the hidden-space slices matter here; free space alone ticking must not redraw the map.
  const hiddenKey = volume?.hidden.map((h) => `${h.kind}:${Math.round(h.bytes / 2 ** 27)}`).join(',') ?? '';
  const tree = useMemo(() => {
    if (!raw) return null;
    const t = withHidden(applyGone(raw, gone), volume);
    return { ...t, children: t.children?.map(merge) };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [raw, gone, hiddenKey]);
  useEffect(() => { setSpaceTree(tree); }, [tree, setSpaceTree]);
  return { tree, loading: loading && !tree };
}

/**
 * Same folders in the same order, the sizes shown as text (the folder and its children) within
 * 1 MB, and nothing deeper moved by more than ~0.05% of the whole.
 */
export function similar(a: TreeNode, b: TreeNode): boolean {
  const tol = Math.max(a.bytes, b.bytes) * 0.0005;
  const walk = (x: TreeNode, y: TreeNode, depth: number): boolean => {
    if (x.path !== y.path || x.kind !== y.kind || x.clutter !== y.clutter) return false;
    if (Math.abs(x.bytes - y.bytes) > (depth <= 1 ? 2 ** 20 : tol)) return false;
    const xc = x.children ?? [], yc = y.children ?? [];
    if (xc.length !== yc.length) return false;
    for (let i = 0; i < xc.length; i++) if (!walk(xc[i], yc[i], depth + 1)) return false;
    return true;
  };
  return walk(a, b, 0);
}
