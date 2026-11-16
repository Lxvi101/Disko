import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import type { TreeNode } from '../../types';
import { splitBytes, catMeta, formatBytes } from '../../lib/format';
import { fileColor, folderColor, restColor } from '../../lib/colors';
import { useStore } from '../../store';

import { RINGS, START_ANGLE, HIDDEN_COLOR, type Datum, type ArcNode, type Layout } from '../../lib/sunburst';
import { ZoomStage, easeOut, useHeldLayout } from './Stage';
export type { Datum } from '../../lib/sunburst';

interface Props {
  tree: TreeNode | null;
  layout: Layout | null;
  loading: boolean;
  canGoUp: boolean;
  hoverPath: string | null;
  colorBy: 'folder' | 'policy';
  stagedPaths: Set<string>;
  onHover: (path: string | null) => void;
  onNavigate: (path: string) => void;
  onRemainder: (path: string) => void;
  onUp: () => void;
  onLayout: (colors: Map<string, string>) => void;
  onContextMenu?: (d: Datum & { total: number }, e: React.MouseEvent) => void;
}

const DEPTH = RINGS.length;
const TWEEN_MS = 240;
type ArcGeom = { x0: number; x1: number; depth: number; color: string };
const RING_SUM = RINGS.reduce((a, b) => a + b, 0);

/** Shared by both disk maps so the legend dots match either view. */
export function nodeColor(d: { data: Datum; depth: number; hue: number }, colorBy: 'folder' | 'policy', light: boolean): string {
  if (d.data.isHidden) return HIDDEN_COLOR;
  if (d.data.isRest) return restColor(light);
  if (colorBy === 'policy') return d.data.kind === 'directory' ? catMeta(d.data.category).color : fileColor(d.depth, light);
  if (d.data.kind !== 'directory') return fileColor(d.depth, light);
  return folderColor(d.hue, d.depth, light);
}

export function cssVar(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** Canvas renderer: one paint per state change, hover repaints only, no per-arc React nodes. */
const Wheel: React.FC<{
  layout: Layout;
  size: number;
  colorBy: 'folder' | 'policy';
  light: boolean;
  stagedPaths: Set<string>;
  hoverPath: string | null;
  onHover: (p: string | null) => void;
  onClick: (d: ArcNode | null) => void;
  onContextMenu?: (d: ArcNode, e: React.MouseEvent) => void;
  canGoUp: boolean;
}> = ({ layout, size, colorBy, light, stagedPaths, hoverPath, onHover, onClick, onContextMenu, canGoUp }) => {
  const ref = useRef<HTMLCanvasElement>(null);
  const R = size / 2 - 6;
  const centerR = R * 0.18;
  const unit = (R - centerR) / RING_SUM;
  const bounds = useMemo(() => {
    const b: { r0: number; r1: number }[] = [{ r0: 0, r1: 0 }];
    let acc = centerR;
    for (let d = 1; d <= DEPTH; d++) {
      const w = RINGS[d - 1] * unit;
      b.push({ r0: acc, r1: acc + w });
      acc += w;
    }
    return b;
  }, [centerR, unit]);

  const stagedSet = useMemo(() => {
    const s = new Set<string>();
    if (!stagedPaths.size) return s;
    const list = [...stagedPaths];
    for (const d of layout.nodes) {
      const p = d.data.path;
      if (stagedPaths.has(p) || list.some((x) => p.startsWith(x + '/'))) s.add(p);
    }
    return s;
  }, [layout, stagedPaths]);

  // Cache colours per layout/theme so hover repaints only touch alpha.
  const fills = useMemo(() => {
    const m = new Map<ArcNode, string>();
    for (const d of layout.nodes) m.set(d, nodeColor(d, colorBy, light));
    return m;
  }, [layout, colorBy, light]);

  // Rearranging (hiding, unhiding, switching detail) tweens arcs from where they are drawn
  // now; a navigation remounts the wheel, so it never tweens between unrelated folders.
  const shown = useRef<Map<string, ArcGeom>>(new Map());
  const tween = useRef<{ from: Map<string, ArcGeom>; start: number } | null>(null);
  useLayoutEffect(() => {
    if (shown.current.size) tween.current = { from: shown.current, start: performance.now() };
  }, [layout]);

  /** Paints one frame; returns true once no tween is running. */
  const draw = useCallback((now: number) => {
    const cv = ref.current;
    if (!cv) return true;
    const dpr = window.devicePixelRatio || 1;
    if (cv.width !== Math.round(size * dpr)) {
      cv.width = Math.round(size * dpr);
      cv.height = Math.round(size * dpr);
    }
    const ctx = cv.getContext('2d');
    if (!ctx) return true;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);
    ctx.translate(size / 2, size / 2);
    const bg = getComputedStyle(cv).getPropertyValue('--bg').trim() || '#292b30';
    const hovered = hoverPath ? layout.byPath.get(hoverPath) ?? null : null;
    const lineage = new Set<string>();
    if (hovered) hovered.ancestors().forEach((a) => lineage.add(a.data.path));
    const descendants = new Set(hovered?.descendants() ?? []);
    const stagedColor = 'hsl(0 0% 60%)';
    const tw = tween.current;
    const p = tw ? Math.min(1, (now - tw.start) / TWEEN_MS) : 1;
    const e = easeOut(p);
    const next = new Map<string, ArcGeom>();
    ctx.lineWidth = 0.6;
    ctx.strokeStyle = bg;
    const paint = (depth: number, x0: number, x1: number, lift: number) => {
      const { r0: baseR0, r1: baseR1 } = bounds[depth];
      const r0 = baseR0 + (depth > 5 ? 2 : 0);
      const r1 = baseR1 + lift;
      const a0 = x0 + START_ANGLE - Math.PI / 2;
      const a1 = x1 + START_ANGLE - Math.PI / 2;
      ctx.beginPath();
      ctx.arc(0, 0, r1, a0, a1);
      ctx.arc(0, 0, r0, a1, a0, true);
      ctx.closePath();
      ctx.fill();
      if (x1 - x0 > 0.004) ctx.stroke();
    };
    for (const d of layout.nodes) {
      const inBranch = descendants.has(d);
      const dim = hovered && !lineage.has(d.data.path) && !inBranch;
      let { x0, x1 } = d;
      if (tw && p < 1) {
        const f = tw.from.get(d.data.path);
        if (f && f.depth === d.depth) {
          x0 = f.x0 + (d.x0 - f.x0) * e;
          x1 = f.x1 + (d.x1 - f.x1) * e;
        } else {
          // New arcs grow out of their own centre.
          const mid = (d.x0 + d.x1) / 2;
          x0 = mid + (d.x0 - mid) * e;
          x1 = mid + (d.x1 - mid) * e;
        }
      }
      const color = stagedSet.has(d.data.path) ? stagedColor : fills.get(d) ?? '#888';
      next.set(d.data.path, { x0, x1, depth: d.depth, color });
      ctx.globalAlpha = dim ? 0.22 : 1;
      ctx.fillStyle = color;
      paint(d.depth, x0, x1, hovered === d ? 3 : 0);
    }
    if (tw && p < 1) {
      // Removed arcs collapse into their centre and fade.
      for (const [path, f] of tw.from) {
        if (layout.byPath.has(path)) continue;
        const mid = (f.x0 + f.x1) / 2;
        ctx.globalAlpha = 1 - e;
        ctx.fillStyle = f.color;
        paint(f.depth, f.x0 + (mid - f.x0) * e, f.x1 + (mid - f.x1) * e, 0);
      }
    }
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.arc(0, 0, centerR, 0, 2 * Math.PI);
    ctx.fillStyle = bg;
    ctx.fill();
    shown.current = next;
    if (p >= 1) tween.current = null;
    return p >= 1;
  }, [layout, size, hoverPath, stagedSet, fills, bounds, centerR]);

  useEffect(() => {
    let id = 0;
    const tick = (now: number) => { if (!draw(now)) id = requestAnimationFrame(tick); };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [draw]);

  const hit = (e: React.MouseEvent): ArcNode | null | 'center' => {
    const rect = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const x = e.clientX - rect.left - size / 2;
    const y = e.clientY - rect.top - size / 2;
    const r = Math.hypot(x, y);
    if (r < centerR) return 'center';
    let depth = 0;
    for (let d = 1; d <= DEPTH; d++) {
      if (r >= bounds[d].r0 && r < bounds[d].r1) {
        depth = d;
        break;
      }
    }
    if (!depth || (depth > 5 && r < bounds[depth].r0 + 2)) return null;
    let theta = Math.atan2(y, x) + Math.PI / 2 - START_ANGLE;
    theta = (theta + 2 * Math.PI) % (2 * Math.PI);
    const arr = layout.byDepth[depth];
    let lo = 0;
    let hi = arr.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const n = arr[mid];
      if (theta < n.x0) hi = mid - 1;
      else if (theta >= n.x1) lo = mid + 1;
      else return n;
    }
    return null;
  };

  const lastHover = useRef<string | null>(null);
  const onMove = (e: React.MouseEvent) => {
    const h = hit(e);
    const p = h && h !== 'center' ? h.data.path : null;
    if (lastHover.current !== p) {
      lastHover.current = p;
      onHover(p);
    }
    (e.currentTarget as HTMLCanvasElement).style.cursor = h === 'center' ? (canGoUp ? 'pointer' : 'default') : h && h.data.kind === 'directory' ? 'pointer' : 'default';
  };

  // Native drag from an arc: same payload as list rows, so the collector accepts it unchanged.
  // Protected arcs still drag; the collector refuses them with the classifier's reason, which is
  // far clearer than a segment that silently will not pick up.
  const onDragStart = (e: React.DragEvent<HTMLCanvasElement>) => {
    const h = hit(e);
    if (!h || h === 'center' || (h.data.isRest || h.data.isHidden)) {
      e.preventDefault();
      return;
    }
    const d = h.data;
    e.dataTransfer.setData('application/x-disko-item', JSON.stringify({ path: d.path, name: d.name, kind: d.kind, total: h.value ?? d.total, category: d.category, reason: d.reason }));
    e.dataTransfer.effectAllowed = 'copy';

    const ghost = document.createElement('div');
    ghost.style.cssText = `position:fixed;top:-1000px;left:-1000px;display:flex;align-items:center;gap:8px;padding:6px 10px;border-radius:8px;font:500 12.5px -apple-system,system-ui,sans-serif;white-space:nowrap;pointer-events:none;background:${cssVar('--bg-2', '#26272c')};color:${cssVar('--text', '#eee')};border:1px solid ${cssVar('--line-2', '#393a41')}`;
    const dot = document.createElement('span');
    dot.style.cssText = `width:8px;height:8px;border-radius:9999px;flex-shrink:0;background:${fills.get(h) ?? '#888'}`;
    const name = document.createElement('span');
    name.textContent = d.name;
    const sz = document.createElement('span');
    sz.textContent = formatBytes(h.value ?? d.total);
    sz.style.cssText = `color:${cssVar('--muted', '#999')};font-variant-numeric:tabular-nums`;
    ghost.append(dot, name, sz);
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 14, 14);
    requestAnimationFrame(() => ghost.remove());
  };

  return (
    <canvas
      ref={ref}
      role="img"
      aria-label="Disk usage sunburst. Use the folder list to navigate, or drag a segment to collect it."
      draggable
      style={{ width: size, height: size, display: 'block' }}
      onDragStart={onDragStart}
      onDragEnd={() => {
        lastHover.current = null;
        onHover(null);
      }}
      onMouseMove={onMove}
      onMouseLeave={() => {
        lastHover.current = null;
        onHover(null);
      }}
      onClick={(e) => {
        const h = hit(e);
        if (h === 'center') onClick(null);
        else if (h && !h.data.isHidden) onClick(h);
      }}
      onContextMenu={(e) => {
        const h = hit(e);
        if (!h || h === 'center' || (h.data.isRest || h.data.isHidden)) return;
        onContextMenu?.(h, e);
      }}
    />
  );
};

export const Sunburst: React.FC<Props> = ({ tree, layout, loading, canGoUp, hoverPath, colorBy, stagedPaths, onHover, onNavigate, onRemainder, onUp, onLayout, onContextMenu }) => {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(640);
  const light = useStore((s) => s.resolvedTheme === 'light');
  const held = useHeldLayout(layout, tree?.path ?? null);
  const shownLayout = held?.layout ?? null;

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize(Math.max(1, Math.floor(Math.min(width, height))));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!shownLayout) return;
    const m = new Map<string, string>();
    for (const d of shownLayout.nodes) m.set(d.data.path, nodeColor(d, colorBy, light));
    onLayout(m);
  }, [shownLayout, colorBy, light, onLayout]);

  const hovered = hoverPath && shownLayout ? shownLayout.byPath.get(hoverPath) ?? null : null;
  const total = shownLayout?.total ?? tree?.total ?? 0;
  const centerValue = hovered ? hovered.value ?? hovered.data.total : total;
  const { value, unit: u } = splitBytes(centerValue);
  const centerR = (size / 2 - 6) * 0.18;

  return (
    // While the next folder loads the previous wheel stays put but stops taking clicks.
    <div ref={wrapRef} className="relative w-full h-full flex items-center justify-center" style={{ pointerEvents: layout ? undefined : 'none' }}>
      {held && (
        <ZoomStage id={held.path} direction={held.direction} width={size} height={size}>
          <Wheel
            layout={held.layout}
            size={size}
            colorBy={colorBy}
            light={light}
            stagedPaths={stagedPaths}
            hoverPath={hoverPath}
            onHover={onHover}
            canGoUp={canGoUp}
            onContextMenu={(d, e) => onContextMenu?.({ ...d.data, total: d.value ?? d.data.total }, e)}
            onClick={(d) => {
              if (!d) return canGoUp && onUp();
              if (d.data.isRest) return onRemainder(d.parent?.data.path ?? d.data.path);
              if (d.data.kind === 'directory') onNavigate(d.data.path);
            }}
          />
        </ZoomStage>
      )}

      <div className="absolute pointer-events-none flex flex-col items-center justify-center text-center" style={{ width: centerR * 2 - 12, height: centerR * 2 - 12, zIndex: 2 }}>
        <div className="flex flex-col items-center leading-none">
          <span className="tnum font-light" style={{ fontSize: Math.max(18, centerR * 0.46) }}>{value}</span>
          <span className="tnum font-light mt-1" style={{ fontSize: Math.max(18, centerR * 0.46) }}>{u}</span>
        </div>
        {loading && (
          <motion.div className="absolute inset-0 rounded-full" style={{ border: '1.5px solid var(--line-2)', borderTopColor: 'var(--text)' }} animate={{ rotate: 360 }} transition={{ repeat: Infinity, duration: 0.9, ease: 'linear' }} />
        )}
      </div>
    </div>
  );
};
