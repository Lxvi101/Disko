import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, partition, type HierarchyRectangularNode } from 'd3-hierarchy';
import { ArrowUp } from 'lucide-react';
import { useStore, sel } from '../../store';
import { splitBytes, parentOf } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { CAT } from '../../lib/categories';
import { wheelColor, type MapNode } from './model';
import { endDrag, openMenu, startDrag } from '../../lib/interact';

const pickable = (n: MapNode) => n.kind === 'dir' || n.kind === 'file';

type Arc = HierarchyRectangularNode<MapNode> & { color: string; outer: number };
interface Layout { arcs: Arc[]; byDepth: Arc[][]; byPath: Map<string, Arc>; root: MapNode }

// Five equal rings you can read, then three thin ones that only show where depth continues.
const MAIN = 5;
const FRINGE = 3;
const RINGS = MAIN + FRINGE;
const FRINGE_W = 0.16;
const SEP = 1.25; // px cut between arcs
const TWEEN = 520;

function own(n: MapNode) {
  return Math.max(0, n.bytes - (n.children?.reduce((a, c) => a + c.bytes, 0) ?? 0));
}

// The overview stays calm: each ring shows a few of a folder's largest items and folds the rest into
// one quiet slice. Opening a folder recomputes this, so everything becomes explorable as you go.
const LIMIT = [8, 12, 10, 8, 6, 4, 3, 2];
const MIN_SHARE = [0, 0.025, 0.035, 0.05, 0.06, 0.09, 0.12, 0.18];
const MIN_TOTAL = [0, 0.0025, 0.003, 0.004, 0.0045, 0.005, 0.006, 0.008];

function simplify(root: MapNode): MapNode {
  const walk = (n: MapNode, depth: number): MapNode => {
    if (!n.children?.length || depth >= RINGS) return n.children ? { ...n, children: undefined } : n;
    const min = Math.max(root.bytes * MIN_TOTAL[depth], n.bytes * MIN_SHARE[depth]);
    const keep: MapNode[] = [];
    const fold: MapNode[] = [];
    let i = 0;
    for (const c of n.children) {
      if (c.kind === 'hidden') keep.push(c);
      else if (c.kind !== 'other' && c.bytes > 0 && c.bytes >= min && i++ < LIMIT[depth]) keep.push(walk(c, depth + 1));
      else fold.push(c);
    }
    if (fold.length) {
      const items = fold.reduce((a, c) => a + (c.kind === 'other' ? Number(/^\d+/.exec(c.name)?.[0] ?? 1) : 1), 0);
      keep.push({
        name: items === 1 ? '1 smaller item' : `${items.toLocaleString()} smaller items`,
        path: `${n.path === '/' ? '' : n.path}/…`, kind: 'other',
        bytes: fold.reduce((a, c) => a + c.bytes, 0), files: fold.reduce((a, c) => a + c.files, 0),
      });
    }
    return { ...n, children: keep };
  };
  return walk(root, 0);
}

function layout(input: MapNode, dark: boolean): Layout {
  const root = hierarchy<MapNode>(simplify(input), (d) => d.children).sum(own);
  root.sort((a, b) => rank(a.data) - rank(b.data) || (b.value ?? 0) - (a.value ?? 0));
  const part = partition<MapNode>().size([2 * Math.PI, RINGS + 1])(root);
  const arcs: Arc[] = [];
  const byDepth: Arc[][] = Array.from({ length: RINGS + 1 }, () => []);
  const byPath = new Map<string, Arc>();
  let branch = 0;
  part.children?.forEach((b) => {
    const idx = b.data.kind === 'dir' || b.data.kind === 'file' ? branch++ : 0;
    const span = Math.max(1e-6, b.x1 - b.x0);
    const spread = Math.min(36, span * 14);
    b.each((d) => {
      if (d.depth < 1 || d.depth > RINGS || d.x1 - d.x0 < 0.0006) return;
      const shift = (((d.x0 + d.x1) / 2 - b.x0) / span - 0.5) * spread;
      const a = d as Arc;
      a.color = wheelColor(d.data, idx, shift, d.depth, dark);
      a.outer = d.depth;
      arcs.push(a);
      byDepth[d.depth].push(a);
      byPath.set(d.data.path, a);
    });
  });
  // How far out each arc's drawn branch reaches, so the clutter halo can sit on the wheel's edge.
  for (let i = arcs.length - 1; i >= 0; i--) {
    const a = arcs[i];
    const p = a.parent as Arc | null;
    if (p && p.depth >= 1 && byPath.get(p.data.path) === p) p.outer = Math.max(p.outer, a.outer);
  }
  for (const arr of byDepth) arr.sort((a, b) => a.x0 - b.x0);
  return { arcs, byDepth, byPath, root: input };
}
/** Did any arc move by more than ~0.4°, or a noticeable arc appear or vanish? */
function visibleChange(prev: Map<string, { x0: number; x1: number; depth: number }>, lay: Layout): boolean {
  const MOVE = 0.007, SIZE = 0.012;
  for (const a of lay.arcs) {
    const p = prev.get(a.data.path);
    if (!p || p.depth !== a.depth) { if (a.x1 - a.x0 > SIZE) return true; continue; }
    if (Math.abs(p.x0 - a.x0) > MOVE || Math.abs(p.x1 - a.x1) > MOVE) return true;
  }
  for (const [path, p] of prev) if (!lay.byPath.has(path) && p.x1 - p.x0 > SIZE) return true;
  return false;
}
const rank = (n: MapNode) => (n.kind === 'hidden' ? 2 : n.kind === 'other' ? 1 : 0);
const ease = (t: number) => 1 - Math.pow(1 - t, 3);
const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export const Sunburst: React.FC<{ tree: MapNode }> = ({ tree }) => {
  const wrap = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(600);
  const dark = useStore((s) => s.dark);
  const spacePath = useStore((s) => s.spacePath);
  const prevPath = useRef(spacePath);
  const dir = spacePath.length >= prevPath.current.length ? 1 : -1;
  useEffect(() => { prevPath.current = spacePath; }, [spacePath]);

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setSize(Math.max(200, Math.floor(Math.min(e.contentRect.width, e.contentRect.height)))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const lay = useMemo(() => layout(tree, dark), [tree, dark]);

  return (
    <div ref={wrap} className="relative w-full h-full flex items-center justify-center">
      <AnimatePresence initial={false} mode="popLayout">
        <motion.div
          key={tree.path}
          initial={reduced() ? false : { opacity: 0, scale: dir > 0 ? 0.55 : 1.4 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: dir > 0 ? 1.5 : 0.6 }}
          transition={{ duration: 0.5, ease: [0.22, 0.8, 0.24, 1] }}
          style={{ width: size, height: size }}
          className="relative"
        >
          <Wheel lay={lay} size={size} />
        </motion.div>
      </AnimatePresence>
    </div>
  );
};

const Wheel: React.FC<{ lay: Layout; size: number }> = ({ lay, size }) => {
  const cv = useRef<HTMLCanvasElement>(null);
  const hover = useStore((s) => s.hover);
  const setHover = useStore((s) => s.setHover);
  const openFolder = useStore((s) => s.openFolder);
  const toggle = useStore((s) => s.toggle);
  const selection = useStore((s) => s.selection);
  const dark = useStore((s) => s.dark);
  const findings = useStore((s) => s.findings);
  const R = size / 2 - 12;
  const inner = R * 0.25;
  const bounds = useMemo(() => {
    const unit = (R - inner - 3) / (MAIN + FRINGE * FRINGE_W);
    const b: { r0: number; r1: number }[] = [{ r0: 0, r1: inner }];
    let acc = inner;
    for (let d = 1; d <= RINGS; d++) {
      const w = (d <= MAIN ? 1 : FRINGE_W) * unit;
      const r0 = acc + (d === MAIN + 1 ? 3 : 0); // a little air before the depth rings
      b.push({ r0, r1: r0 + w });
      acc = r0 + w;
    }
    return b;
  }, [R, inner]);

  // Clutter halo: a thin line on the wheel's edge showing where reclaimable space sits, by category.
  const halo = useMemo(() => {
    const per = new Map<Arc, Map<string, number>>();
    for (const f of Object.values(findings)) {
      for (const p of f.paths) {
        let best: Arc | undefined;
        let cur = p;
        while (cur && cur !== '/') {
          const a = lay.byPath.get(cur);
          if (a) { best = a; break; }
          cur = cur.slice(0, cur.lastIndexOf('/'));
        }
        if (!best) continue;
        const m = per.get(best) ?? new Map<string, number>();
        m.set(f.category, (m.get(f.category) ?? 0) + f.bytes / f.paths.length);
        per.set(best, m);
      }
    }
    const segs: { x0: number; x1: number; r: number; color: string; path: string }[] = [];
    for (const [a, m] of per) {
      const v = a.value || a.data.bytes || 1;
      let x = a.x0;
      for (const [cat, b] of [...m].sort((p, q) => q[1] - p[1])) {
        const w = Math.min(a.x1 - x, ((a.x1 - a.x0) * b) / v);
        if (w <= 0) break;
        segs.push({ x0: x, x1: x + w, r: bounds[a.outer].r1 + 5, color: CAT[cat as keyof typeof CAT].color, path: a.data.path });
        x += w;
      }
    }
    return segs;
  }, [findings, lay, bounds]);

  const selected = useMemo(() => {
    const paths = [...selection.values()].flatMap((i) => i.paths);
    return (p: string) => paths.some((s) => p === s || p.startsWith(s + '/'));
  }, [selection]);

  // Live growth: arcs tween from where they were drawn last.
  const shown = useRef(new Map<string, { x0: number; x1: number; depth: number; color: string }>());
  const tween = useRef<{ from: Map<string, { x0: number; x1: number; depth: number; color: string }>; t0: number } | null>(null);
  // Only animate changes you can see; small live updates (a cache growing) just redraw.
  useLayoutEffect(() => {
    if (!shown.current.size || reduced() || !visibleChange(shown.current, lay)) return;
    tween.current = { from: shown.current, t0: performance.now() };
  }, [lay]);

  const draw = useCallback((now: number) => {
    const c = cv.current;
    if (!c) return true;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== Math.round(size * dpr)) { c.width = Math.round(size * dpr); c.height = Math.round(size * dpr); }
    const ctx = c.getContext('2d')!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);
    ctx.translate(size / 2, size / 2);
    const tw = tween.current;
    const p = tw ? Math.min(1, (now - tw.t0) / TWEEN) : 1;
    const e = ease(p);
    const h = hover ? lay.byPath.get(hover) : undefined;
    const lineage = new Set<Arc>();
    if (h) { h.ancestors().forEach((a) => lineage.add(a as Arc)); h.descendants().forEach((a) => lineage.add(a as Arc)); }
    const next = new Map<string, { x0: number; x1: number; depth: number; color: string }>();

    const sector = (depth: number, x0: number, x1: number, lift: number) => {
      const { r0, r1 } = bounds[depth];
      const a0 = x0 - Math.PI / 2, a1 = x1 - Math.PI / 2;
      ctx.beginPath();
      ctx.arc(0, 0, r1 + lift, a0, a1);
      ctx.arc(0, 0, r0, a1, a0, true);
      ctx.closePath();
    };
    // Fill everything first, then cut thin gaps between arcs wide enough to tell apart. Slivers
    // too small to read stay one calm band instead of a comb of hairlines.
    const cuts: [number, number, number, number][] = [];
    const fill = (depth: number, x0: number, x1: number, color: string, alpha: number, lift: number, picked: boolean) => {
      sector(depth, x0, x1, lift);
      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.fill();
      if (picked) { ctx.fillStyle = 'rgba(255,255,255,.34)'; ctx.fill(); }
      if ((x1 - x0) * bounds[depth].r1 > 3.5) cuts.push([depth, x0, x1, lift]);
    };

    let lifted: [number, number, number, string, boolean] | null = null;
    for (const a of lay.arcs) {
      let { x0, x1 } = a;
      if (tw && p < 1) {
        const f = tw.from.get(a.data.path);
        if (f && f.depth === a.depth) { x0 = f.x0 + (a.x0 - f.x0) * e; x1 = f.x1 + (a.x1 - f.x1) * e; }
        else { const m = (a.x0 + a.x1) / 2; x0 = m + (a.x0 - m) * e; x1 = m + (a.x1 - m) * e; }
      }
      next.set(a.data.path, { x0, x1, depth: a.depth, color: a.color });
      if (a === h) { lifted = [a.depth, x0, x1, a.color, selected(a.data.path)]; continue; }
      fill(a.depth, x0, x1, a.color, h && !lineage.has(a) ? 0.22 : 1, 0, selected(a.data.path));
    }
    if (tw && p < 1) {
      for (const [path, f] of tw.from) {
        if (lay.byPath.has(path)) continue;
        const m = (f.x0 + f.x1) / 2;
        fill(f.depth, f.x0 + (m - f.x0) * e, f.x1 + (m - f.x1) * e, f.color, 1 - e, 0, false);
      }
    }
    if (lifted) fill(lifted[0], lifted[1], lifted[2], lifted[3], 1, 4, lifted[4]);
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = 'destination-out';
    ctx.lineWidth = SEP;
    ctx.lineJoin = 'miter';
    ctx.strokeStyle = '#000';
    for (const [depth, x0, x1, lift] of cuts) { sector(depth, x0, x1, lift); ctx.stroke(); }
    // Soft light falling from above, matching the window's bloom.
    ctx.globalCompositeOperation = 'source-atop';
    const g = ctx.createLinearGradient(0, -R, 0, R);
    g.addColorStop(0, dark ? 'rgba(255,255,255,.06)' : 'rgba(255,255,255,.14)');
    g.addColorStop(0.6, 'rgba(255,255,255,0)');
    g.addColorStop(1, dark ? 'rgba(0,0,0,.12)' : 'rgba(0,0,0,.03)');
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(0, 0, R + 6, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalCompositeOperation = 'source-over';
    // Halo, hugging the outline of the branch that holds the clutter.
    ctx.lineCap = 'round';
    ctx.lineWidth = 3;
    for (const sgm of halo) {
      const r = sgm.r;
      const pad = Math.min(0.004, (sgm.x1 - sgm.x0) / 4);
      const a0 = sgm.x0 - Math.PI / 2 + pad, a1 = sgm.x1 - Math.PI / 2 - pad;
      if ((a1 - a0) * r < 1) continue;
      ctx.beginPath();
      ctx.arc(0, 0, r, a0, a1);
      ctx.strokeStyle = sgm.color;
      ctx.globalAlpha = h && !(hover && (sgm.path === hover || sgm.path.startsWith(hover + '/'))) ? 0.25 : 0.95;
      ctx.stroke();
    }
    ctx.lineCap = 'butt';
    ctx.globalAlpha = 1;
    shown.current = next;
    if (p >= 1) tween.current = null;
    return p >= 1;
  }, [lay, size, hover, bounds, selected, R, dark, halo]);

  useEffect(() => {
    let id = 0;
    const tick = (t: number) => { if (!draw(t)) id = requestAnimationFrame(tick); };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [draw]);

  const hit = (e: React.MouseEvent): Arc | 'center' | null => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - r.left - size / 2;
    const y = e.clientY - r.top - size / 2;
    const d = Math.hypot(x, y);
    if (d < inner) return 'center';
    let depth = 0;
    // Thin depth rings get a forgiving target.
    for (let i = 1; i <= RINGS; i++) if (d >= bounds[i].r0 - 1.5 && d < bounds[i].r1 + (i > MAIN ? 3 : 1)) { depth = i; break; }
    if (!depth) return null;
    const theta = (Math.atan2(y, x) + Math.PI / 2 + 2 * Math.PI) % (2 * Math.PI);
    const arr = lay.byDepth[depth];
    let lo = 0, hi = arr.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const n = arr[mid];
      if (theta < n.x0) hi = mid - 1;
      else if (theta >= n.x1) lo = mid + 1;
      else return n;
    }
    return null;
  };

  const [overCenter, setOverCenter] = useState(false);
  const canUp = lay.root.path !== '/';
  const h = hover ? lay.byPath.get(hover) : undefined;
  const shownNode: MapNode = h?.data ?? lay.root;
  const s = splitBytes(h ? h.value ?? h.data.bytes : lay.root.bytes);

  return (
    <div className="relative w-full h-full">
      <canvas
        ref={cv}
        style={{ width: size, height: size, display: 'block' }}
        role="img"
        aria-label="Sunburst of the current folder"
        onMouseMove={(e) => {
          const t = hit(e);
          setOverCenter(t === 'center');
          setHover(t && t !== 'center' ? t.data.path : null);
          (e.currentTarget as HTMLElement).style.cursor = t === 'center' ? (canUp ? 'pointer' : 'default') : t && t.data.kind === 'dir' ? 'pointer' : 'default';
        }}
        onMouseLeave={() => { setHover(null); setOverCenter(false); }}
        draggable
        onDragStart={(e) => {
          const t = hit(e);
          if (!t || t === 'center' || !pickable(t.data) || t.data.protected) return e.preventDefault();
          startDrag(e, sel.node(t.data));
        }}
        onDragEnd={endDrag}
        onContextMenu={(e) => {
          const t = hit(e);
          if (!t || t === 'center' || !pickable(t.data)) return e.preventDefault();
          openMenu(e, sel.node(t.data), { folder: t.data.kind === 'dir' ? t.data.path : undefined });
        }}
        onClick={(e) => {
          const t = hit(e);
          if (t === 'center') { const up = parentOf(lay.root.path); if (up) openFolder(up); return; }
          if (!t || t.data.kind === 'hidden' || t.data.kind === 'other') return;
          if (e.metaKey || e.altKey || t.data.kind === 'file') { if (!t.data.protected) toggle(sel.node(t.data)); }
          else openFolder(t.data.path);
        }}
      />
      <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
        <div className="flex flex-col items-center text-center" style={{ width: inner * 1.7 }}>
          {overCenter && canUp ? (
            <div className="flex flex-col items-center gap-1 muted"><ArrowUp size={30} /><span className="text-[14px] font-semibold">Back</span></div>
          ) : (
            <>
              <NodeIcon node={shownNode === lay.root && lay.root.path === '/' ? { ...shownNode, kind: 'disk' } : shownNode} size={Math.min(44, inner * 0.32)} />
              <div className="mt-2 font-medium truncate max-w-full muted" style={{ fontSize: Math.max(13, Math.min(16, inner * 0.11)) }}>{shownNode.name}</div>
              <div className="flex items-baseline gap-1 mt-1">
                <span className="num font-semibold leading-none tracking-[-0.04em]" style={{ fontSize: Math.max(24, Math.min(46, inner * 0.3)) }}>{s.value}</span>
                <span className="num unit" style={{ fontSize: Math.max(13, inner * 0.11) }}>{s.unit}</span>
              </div>
              {shownNode.why && <div className="muted text-[12.5px] mt-1.5 leading-snug">{shownNode.why}</div>}
            </>
          )}
        </div>
      </div>
    </div>
  );
};
