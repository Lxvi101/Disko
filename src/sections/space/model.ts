import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { catOf } from '../../lib/categories';
import { applyGone, useStore } from '../../store';
import { parentOf } from '../../lib/format';
import type { TreeNode, VolumeInfo } from '../../lib/types';

/** A tree node plus what the map needs to draw it. */
export type MapNode = TreeNode & { hue?: number; why?: string; /** The outermost folder of a merged chain ("Users › alex"), for stable keys. */ from?: string };

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

// The sunburst gives each top-level branch its own muted tone, so you can follow a folder outward.
const WHEEL_HUES = [214, 168, 262, 24, 332, 140, 44, 192, 290, 8];

/** Sunburst fill: the branch tone, calmer and lighter with depth; files quieter, folded items nearly neutral. */
export function wheelColor(n: TreeNode, branch: number, shift: number, depth: number, dark: boolean): string {
  if (n.kind === 'hidden') return dark ? '#45434f' : '#d0ced9';
  const hue = (WHEEL_HUES[branch % WHEEL_HUES.length] + shift + 360) % 360;
  const d = Math.min(depth, 6) - 1;
  if (n.kind === 'other') return dark ? `hsl(${hue} 12% ${31 + d * 2.5}%)` : `hsl(${hue} 12% ${84 - d * 1.5}%)`;
  const file = n.kind === 'file';
  const s = (dark ? 38 - d * 3 : 42 - d * 3) * (file ? 0.45 : 1);
  const l = dark ? 50 + d * 4.5 - (file ? 6 : 0) : 60 + d * 4 + (file ? 4 : 0);
  return `hsl(${hue} ${s.toFixed(1)}% ${Math.min(l, 84).toFixed(1)}%)`;
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

/**
 * The engine names a folder's folded small items with the folder's own path; give them their own
 * (`folder/…`) so hover, keys and the clutter halo never mistake them for the folder.
 */
export function distinctOther<T extends TreeNode>(n: T): T {
  if (!n.children) return n;
  const base = n.path === '/' ? '' : n.path;
  return { ...n, children: n.children.map((c) => (c.kind === 'other' && c.path === n.path ? { ...c, path: `${base}/…` } : distinctOther(c))) };
}

/** A folder whose only content is one folder shows as one item ("Users › alex"); opening it goes straight in. */
export function merge(n: MapNode): MapNode {
  let cur = n;
  const names = [n.name];
  while (cur.children?.length === 1 && cur.children[0].kind === 'dir' && cur.children[0].bytes >= cur.bytes * 0.98) {
    cur = cur.children[0];
    names.push(cur.name);
  }
  const children = cur.children?.map(merge);
  return cur === n ? (children ? { ...n, children } : n) : { ...cur, name: names.join(' › '), children, from: n.path };
}

export const isNavigable = (n: TreeNode) => n.kind === 'dir';

/** Depth each view needs below the current folder. */
export const DEPTH = { sunburst: 8, bubbles: 2, blocks: 3, list: 1 } as const;

/**
 * Fetches the current folder at the depth the view needs, keeps it fresh while the scan streams
 * (at most ~1/s) and after removals, and applies optimistic removals on top.
 */
export function useSpaceTree(): { tree: MapNode | null; loading: boolean; error: string | null; retry: () => void } {
  const path = useStore((s) => s.spacePath);
  const view = useStore((s) => s.spaceView);
  const version = useStore((s) => s.treeVersion);
  const scanTick = useStore((s) => s.scanTick);
  const gone = useStore((s) => s.gone);
  const volume = useStore((s) => s.volume);
  const setSpaceTree = useStore((s) => s.setSpaceTree);
  const cache = useStore((s) => s.treeCache);
  const depth = DEPTH[view];
  const key = `${path}|${depth}`;
  // Coming back to Space (or to a folder seen before) shows the last tree at once; a refetch follows.
  const [raw, setRaw] = useState<TreeNode | null>(() => cache.get(key) ?? null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const last = useRef(0);
  const timer = useRef(0);
  const keep = (t: TreeNode) => { cache.set(key, t); if (cache.size > 40) cache.delete(cache.keys().next().value!); };

  // Navigation and view changes fetch right away.
  useEffect(() => {
    let live = true;
    const hit = cache.get(key);
    if (hit) setRaw(hit);
    setLoading(true);
    setError(null);
    const fail = (why: string) => {
      if (!live) return;
      setLoading(false);
      // A folder that vanished (removed elsewhere) sends us to its parent.
      const up = parentOf(path);
      if (up) useStore.getState().openFolder(up);
      else setError(why);
    };
    api.tree(path === '/' ? null : path, depth).then((t) => {
      if (!live) return;
      if (!t) return fail('Disko has no picture of this disk yet.');
      keep(t);
      setRaw(t);
      setLoading(false);
      last.current = performance.now();
    }).catch((e) => fail(String(e)));
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, depth, attempt]);

  // Removals, undo, outside changes and the streaming scan: refetch at most ~1/s, and keep the
  // current tree when nothing visible changed so views don't re-layout or re-animate.
  const accept = (t: TreeNode | null) => {
    if (!t) return;
    keep(t);
    setError(null);
    setRaw((old) => (old && similar(old, t) ? old : t));
  };
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
    const t = withHidden(distinctOther(applyGone(raw, gone)), volume);
    return { ...t, name: t.path === '/' ? volume?.name || 'Macintosh HD' : t.name, children: t.children?.map(merge) };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [raw, gone, hiddenKey]);
  useEffect(() => { setSpaceTree(tree); }, [tree, setSpaceTree]);
  return { tree, loading: loading && !tree, error: tree ? null : error, retry: () => setAttempt((a) => a + 1) };
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
