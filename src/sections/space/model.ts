import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { catOf } from '../../lib/categories';
import { applyGone, useStore } from '../../store';
import type { TreeNode, VolumeInfo } from '../../lib/types';

/** A tree node plus what the map needs to draw it. */
export type MapNode = TreeNode & { hue?: number; why?: string };

const BRANCH_HUES = [226, 262, 192, 292, 210, 246, 172, 316];

/** Calm hues for ordinary folders, vivid category colours for clutter. */
export function colorOf(n: TreeNode, hue: number, depth: number, dark: boolean): string {
  const cat = catOf(n.clutter);
  if (cat) return cat.color;
  if (n.kind === 'hidden') return dark ? '#6f6596' : '#a9a1c9';
  if (n.kind === 'other') return dark ? '#3a3f55' : '#c3c7d8';
  const file = n.kind === 'file';
  const s = file ? 18 : 42;
  const l = dark ? 46 + depth * 5 + (file ? -6 : 0) : 66 + depth * 3 + (file ? 6 : 0);
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

  const refetch = () => {
    last.current = performance.now();
    api.tree(path === '/' ? null : path, depth).then(setRaw).catch(() => {});
  };

  // Removals, undo and outside changes: show them now.
  useEffect(() => {
    if (version) refetch();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  // The streaming scan: at most ~1 fetch per second.
  useEffect(() => {
    if (!scanTick) return;
    const run = () => {
      last.current = performance.now();
      api.tree(path === '/' ? null : path, depth).then(setRaw).catch(() => {});
    };
    const since = performance.now() - last.current;
    clearTimeout(timer.current);
    if (since > 900) run();
    else timer.current = window.setTimeout(run, 900 - since);
    return () => clearTimeout(timer.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scanTick]);

  // While the next folder loads, the previous one stays up so views can morph between them.
  const tree = useMemo(() => {
    if (!raw) return null;
    const t = withHidden(applyGone(raw, gone), volume);
    return { ...t, children: t.children?.map(merge) };
  }, [raw, gone, volume]);
  useEffect(() => { setSpaceTree(tree); }, [tree, setSpaceTree]);
  return { tree, loading: loading && !tree };
}
