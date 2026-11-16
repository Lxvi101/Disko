import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, partition, type HierarchyRectangularNode } from 'd3-hierarchy';
import { ArrowUp } from 'lucide-react';
import { useStore, sel } from '../../store';
import { splitBytes, parentOf } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { CAT, catOf } from '../../lib/categories';
import { colorOf, hueFor, type MapNode } from './model';

type Arc = HierarchyRectangularNode<MapNode> & { color: string };
interface Layout { arcs: Arc[]; byDepth: Arc[][]; byPath: Map<string, Arc>; root: MapNode }

const RING_W = [1, 0.86, 0.72, 0.58];
const RINGS = RING_W.length;
const GAP = 1.3; // px between arcs
const TWEEN = 520;

function own(n: MapNode) {
  return Math.max(0, n.bytes - (n.children?.reduce((a, c) => a + c.bytes, 0) ?? 0));
}

function layout(input: MapNode, dark: boolean): Layout {
  const tree = input;
  const root = hierarchy<MapNode>(tree, (d) => d.children).sum(own);
  root.sort((a, b) => rank(a.data) - rank(b.data) || (b.value ?? 0) - (a.value ?? 0));
  const part = partition<MapNode>().size([2 * Math.PI, RINGS + 1])(root);
  const arcs: Arc[] = [];
  const byDepth: Arc[][] = Array.from({ length: RINGS + 1 }, () => []);
  const byPath = new Map<string, Arc>();
  part.children?.forEach((b, i) => {
    const hue = hueFor(i);
    b.each((d) => {
      if (d.depth < 1 || d.depth > RINGS || d.x1 - d.x0 < 0.0025) return;
      const shift = ((d.x0 + d.x1) / 2 - b.x0) / Math.max(1e-6, b.x1 - b.x0) - 0.5;
      const a = d as Arc;
      a.color = colorOf(d.data, hue + shift * 26, d.depth, dark);
      arcs.push(a);
      byDepth[d.depth].push(a);
      byPath.set(d.data.path, a);
    });
  });
  return { arcs, byDepth, byPath, root: input };
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
      <div className="absolute rounded-full pointer-events-none" style={{ width: size * 0.9, height: size * 0.9, background: 'radial-gradient(circle, rgba(141,123,255,.16), transparent 65%)', filter: 'blur(20px)' }} />
      <AnimatePresence initial={false} mode="popLayout">
        <motion.div
          key={tree.path}
          initial={reduced() ? false : { opacity: 0, scale: dir > 0 ? 0.55 : 1.4 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: dir > 0 ? 1.5 : 0.6 }}
          transition={{ duration: 0.42, ease: [0.2, 0.8, 0.2, 1] }}
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
  const R = size / 2 - 22;
  const findings = useStore((s) => s.findings);
  const inner = R * 0.32;
  const bounds = useMemo(() => {
    const total = RING_W.reduce((a, b) => a + b, 0);
    const unit = (R - inner) / total;
    const b: { r0: number; r1: number }[] = [{ r0: 0, r1: inner }];
    let acc = inner;
    for (const w of RING_W) { b.push({ r0: acc + GAP, r1: acc + w * unit }); acc += w * unit; }
    return b;
  }, [R, inner]);

  // Clutter halo: an outer orbit showing exactly where reclaimable space sits, by category.
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
    const segs: { x0: number; x1: number; color: string; path: string }[] = [];
    for (const [a, m] of per) {
      const v = a.value || a.data.bytes || 1;
      let x = a.x0;
      for (const [cat, b] of [...m].sort((p, q) => q[1] - p[1])) {
        const w = Math.min(a.x1 - x, ((a.x1 - a.x0) * b) / v);
        if (w <= 0) break;
        segs.push({ x0: x, x1: x + w, color: CAT[cat as keyof typeof CAT].color, path: a.data.path });
        x += w;
      }
    }
    return segs;
  }, [findings, lay]);

  const selected = useMemo(() => {
    const paths = [...selection.values()].flatMap((i) => i.paths);
    return (p: string) => paths.some((s) => p === s || p.startsWith(s + '/'));
  }, [selection]);

  // Live growth: arcs tween from where they were drawn last.
  const shown = useRef(new Map<string, { x0: number; x1: number; depth: number; color: string }>());
  const tween = useRef<{ from: Map<string, { x0: number; x1: number; depth: number; color: string }>; t0: number } | null>(null);
  useLayoutEffect(() => {
    if (shown.current.size && !reduced()) tween.current = { from: shown.current, t0: performance.now() };
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

    const paint = (depth: number, x0: number, x1: number, color: string, lift: number, alpha: number, picked: boolean) => {
      const { r0, r1: rb } = bounds[depth];
      const r1 = rb + lift;
      const a0 = x0 - Math.PI / 2;
      const a1 = x1 - Math.PI / 2;
      const span = (a1 - a0) * r0 - GAP * 2;
      if (span <= 0.3 && (a1 - a0) * r1 - GAP * 2 <= 0.6) return;
      const cr = Math.max(0, Math.min(5, (r1 - r0) / 2 - 0.5, span / 2 - 0.5));
      const g = GAP + cr;
      const ir = r0 + cr;
      const or = r1 - cr;
      const ia0 = a0 + g / ir, ia1 = a1 - g / ir;
      const oa0 = a0 + g / or, oa1 = a1 - g / or;
      ctx.beginPath();
      ctx.arc(0, 0, or, oa0, Math.max(oa0, oa1));
      if (ia1 > ia0) ctx.arc(0, 0, ir, ia1, ia0, true);
      else ctx.lineTo(Math.cos((a0 + a1) / 2) * ir, Math.sin((a0 + a1) / 2) * ir);
      ctx.closePath();
      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.strokeStyle = color;
      ctx.lineJoin = 'round';
      ctx.lineWidth = cr * 2;
      ctx.fill();
      if (cr > 0) ctx.stroke();
      if (picked) {
        ctx.fillStyle = 'rgba(255,255,255,.42)';
        ctx.strokeStyle = 'rgba(255,255,255,.42)';
        ctx.fill();
        if (cr > 0) ctx.stroke();
      }
    };

    for (const a of lay.arcs) {
      let { x0, x1 } = a;
      if (tw && p < 1) {
        const f = tw.from.get(a.data.path);
        if (f && f.depth === a.depth) { x0 = f.x0 + (a.x0 - f.x0) * e; x1 = f.x1 + (a.x1 - f.x1) * e; }
        else { const m = (a.x0 + a.x1) / 2; x0 = m + (a.x0 - m) * e; x1 = m + (a.x1 - m) * e; }
      }
      next.set(a.data.path, { x0, x1, depth: a.depth, color: a.color });
      const dim = h && !lineage.has(a);
      paint(a.depth, x0, x1, a.color, a === h ? 6 : 0, 1, selected(a.data.path));
      // Dim by veiling with the background, so rounded edges don't double up.
      if (dim) paint(a.depth, x0, x1, dark ? 'rgba(6,7,14,.74)' : 'rgba(238,240,247,.74)', 0, 1, false);
    }
    if (tw && p < 1) {
      for (const [path, f] of tw.from) {
        if (lay.byPath.has(path)) continue;
        const m = (f.x0 + f.x1) / 2;
        paint(f.depth, f.x0 + (m - f.x0) * e, f.x1 + (m - f.x1) * e, f.color, 0, 1 - e, false);
      }
    }
    // Halo.
    ctx.globalAlpha = 1;
    for (const sgm of halo) {
      if (sgm.x1 - sgm.x0 < 0.002) continue;
      const a0 = sgm.x0 - Math.PI / 2 + 0.004, a1 = sgm.x1 - Math.PI / 2 - 0.004;
      if (a1 <= a0) continue;
      ctx.beginPath();
      ctx.arc(0, 0, R + 11, a0, a1);
      ctx.strokeStyle = sgm.color;
      ctx.lineWidth = 7;
      ctx.lineCap = 'round';
      ctx.globalAlpha = h && !(hover && (sgm.path === hover || sgm.path.startsWith(hover + '/'))) ? 0.3 : 1;
      ctx.shadowColor = sgm.color;
      ctx.shadowBlur = 12;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }
    ctx.lineCap = 'butt';
    // Glossy sheen over the wheel.
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = 'source-atop';
    const g = ctx.createRadialGradient(-R * 0.35, -R * 0.45, R * 0.05, 0, 0, R * 1.1);
    g.addColorStop(0, dark ? 'rgba(255,255,255,.22)' : 'rgba(255,255,255,.35)');
    g.addColorStop(0.55, 'rgba(255,255,255,0)');
    g.addColorStop(1, dark ? 'rgba(0,0,0,.18)' : 'rgba(0,0,0,.05)');
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(0, 0, R + 1, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalCompositeOperation = 'source-over';
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
    for (let i = 1; i <= RINGS; i++) if (d >= bounds[i].r0 - GAP && d < bounds[i].r1 + 1) { depth = i; break; }
    if (!depth) return null;
    const theta = (Math.atan2(y, x) + Math.PI / 2 + 2 * Math.PI) % (2 * Math.PI);
    const arr = lay.byDepth[depth];
    let lo = 0, hi = arr.length - 1;
    const sorted = arr; // partition order is angular within depth
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const n = sorted[mid];
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
  const cat = catOf(shownNode.clutter);

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
        onClick={(e) => {
          const t = hit(e);
          if (t === 'center') { const up = parentOf(lay.root.path); if (up) openFolder(up); return; }
          if (!t || t.data.kind === 'hidden' || t.data.kind === 'other') return;
          if (e.metaKey || e.altKey || t.data.kind === 'file') toggle(sel.node(t.data));
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
              <div className="mt-2 font-semibold truncate max-w-full" style={{ fontSize: Math.max(13, Math.min(17, inner * 0.12)), color: cat?.color }}>{shownNode.name}</div>
              <div className="flex items-baseline gap-1 mt-0.5">
                <span className="num font-bold leading-none" style={{ fontSize: Math.max(24, Math.min(46, inner * 0.3)) }}>{s.value}</span>
                <span className="num font-semibold muted" style={{ fontSize: Math.max(13, inner * 0.11) }}>{s.unit}</span>
              </div>
              {shownNode.why && <div className="muted text-[12.5px] mt-1.5 leading-snug">{shownNode.why}</div>}
            </>
          )}
        </div>
      </div>
    </div>
  );
};
