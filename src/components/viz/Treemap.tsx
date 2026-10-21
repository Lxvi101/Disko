import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { formatBytes } from '../../lib/format';
import { inkFor } from '../../lib/colors';
import { useStore } from '../../store';
import type { Datum } from '../../lib/sunburst';
import { HEADER, blockAt, layoutBlocks, type Block, type BlockLayout } from '../../lib/treemap';
import { nodeColor, cssVar } from './Sunburst';
import { ZoomStage, easeOut, useHeldLayout } from './Stage';
import { Spinner } from '../ui';

interface Props {
  /** Drawable tree for the current folder; null while it loads. */
  datum: Datum | null;
  path: string | null;
  loading: boolean;
  hoverPath: string | null;
  colorBy: 'folder' | 'policy';
  stagedPaths: Set<string>;
  onHover: (path: string | null) => void;
  onNavigate: (path: string) => void;
  onRemainder: (path: string) => void;
  onLayout: (colors: Map<string, string>) => void;
  onContextMenu?: (d: Datum & { total: number }, e: React.MouseEvent) => void;
}

const TWEEN_MS = 240;
const FONT = '500 11px -apple-system, BlinkMacSystemFont, system-ui, sans-serif';
type RectGeom = { x0: number; y0: number; x1: number; y1: number; color: string };

const Blocks: React.FC<{
  layout: BlockLayout;
  colorBy: 'folder' | 'policy';
  light: boolean;
  stagedPaths: Set<string>;
  hoverPath: string | null;
  onHover: (p: string | null) => void;
  onClick: (d: Block) => void;
  onContextMenu?: (d: Block, e: React.MouseEvent) => void;
}> = ({ layout, colorBy, light, stagedPaths, hoverPath, onHover, onClick, onContextMenu }) => {
  const ref = useRef<HTMLCanvasElement>(null);
  const { width, height } = layout;

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

  const fills = useMemo(() => {
    const m = new Map<Block, string>();
    for (const d of layout.nodes) m.set(d, nodeColor(d, colorBy, light));
    return m;
  }, [layout, colorBy, light]);

  // Same contract as the wheel: rearranging tweens from what is on screen; resizing snaps.
  const shown = useRef<Map<string, RectGeom>>(new Map());
  const shownSize = useRef({ width, height });
  const tween = useRef<{ from: Map<string, RectGeom>; start: number } | null>(null);
  useLayoutEffect(() => {
    const resized = shownSize.current.width !== width || shownSize.current.height !== height;
    shownSize.current = { width, height };
    tween.current = shown.current.size && !resized ? { from: shown.current, start: performance.now() } : null;
  }, [layout, width, height]);

  const draw = useCallback((now: number) => {
    const cv = ref.current;
    if (!cv) return true;
    const dpr = window.devicePixelRatio || 1;
    if (cv.width !== Math.round(width * dpr) || cv.height !== Math.round(height * dpr)) {
      cv.width = Math.round(width * dpr);
      cv.height = Math.round(height * dpr);
    }
    const ctx = cv.getContext('2d');
    if (!ctx) return true;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const hovered = hoverPath ? layout.byPath.get(hoverPath) ?? null : null;
    const lineage = new Set<Block>(hovered?.ancestors() as Block[] | undefined);
    const branch = new Set<Block>(hovered?.descendants() as Block[] | undefined);
    const tw = tween.current;
    const p = tw ? Math.min(1, (now - tw.start) / TWEEN_MS) : 1;
    const e = easeOut(p);
    const next = new Map<string, RectGeom>();
    const labels: { x: number; y: number; w: number; h: number; name: string; size: string; ink: string; header: boolean; alpha: number }[] = [];
    let hoverRect: RectGeom | null = null;

    for (const d of layout.nodes) {
      let { x0, y0, x1, y1 } = d;
      if (tw && p < 1) {
        const f = tw.from.get(d.data.path);
        if (f) {
          x0 = f.x0 + (d.x0 - f.x0) * e; y0 = f.y0 + (d.y0 - f.y0) * e;
          x1 = f.x1 + (d.x1 - f.x1) * e; y1 = f.y1 + (d.y1 - f.y1) * e;
        } else {
          const cx = (d.x0 + d.x1) / 2, cy = (d.y0 + d.y1) / 2;
          x0 = cx + (d.x0 - cx) * e; y0 = cy + (d.y0 - cy) * e;
          x1 = cx + (d.x1 - cx) * e; y1 = cy + (d.y1 - cy) * e;
        }
      }
      const color = stagedSet.has(d.data.path) ? 'hsl(0 0% 60%)' : fills.get(d) ?? '#888';
      const g = { x0, y0, x1, y1, color };
      next.set(d.data.path, g);
      if (d === hovered) hoverRect = g;
      const dim = hovered && !lineage.has(d) && !branch.has(d);
      const alpha = dim ? 0.3 : 1;
      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
      const w = x1 - x0, h = y1 - y0;
      const name = d.data.isRest ? 'smaller objects' : d.data.name;
      if (d.children && d.header >= HEADER && w > 44) labels.push({ x: x0, y: y0, w, h: HEADER, name, size: formatBytes(d.value ?? d.data.total), ink: inkFor(color), header: true, alpha });
      else if (!d.children && w > 54 && h > 30) labels.push({ x: x0, y: y0, w, h, name, size: formatBytes(d.value ?? d.data.total), ink: inkFor(color), header: false, alpha });
    }
    if (tw && p < 1) {
      for (const [path, f] of tw.from) {
        if (layout.byPath.has(path)) continue;
        const cx = (f.x0 + f.x1) / 2, cy = (f.y0 + f.y1) / 2;
        ctx.globalAlpha = 1 - e;
        ctx.fillStyle = f.color;
        ctx.fillRect(f.x0 + (cx - f.x0) * e, f.y0 + (cy - f.y0) * e, (f.x1 - f.x0) * (1 - e), (f.y1 - f.y0) * (1 - e));
      }
    }

    // Labels fade in once the blocks have settled; mid-tween text would smear.
    ctx.font = FONT;
    ctx.textBaseline = 'middle';
    for (const l of labels) {
      // The size only shows when the name keeps a readable share; the name never runs under it.
      const sw = l.header ? ctx.measureText(l.size).width : 0;
      const room = l.header && l.w - 8 - sw - 10 > Math.min(ctx.measureText(l.name).width, 60);
      ctx.save();
      ctx.globalAlpha = l.alpha * (tw && p < 1 ? e * e : 1);
      ctx.beginPath();
      ctx.rect(l.x + 4, l.y, l.w - 8 - (room ? sw + 10 : 0), l.h);
      ctx.clip();
      ctx.fillStyle = l.ink;
      if (l.header) {
        ctx.fillText(l.name, l.x + 5, l.y + HEADER / 2 + 0.5);
        ctx.restore();
        ctx.save();
        ctx.globalAlpha = l.alpha * (tw && p < 1 ? e * e : 1);
        ctx.fillStyle = l.ink;
        if (room) {
          ctx.globalAlpha *= 0.7;
          ctx.fillText(l.size, l.x + l.w - 5 - sw, l.y + HEADER / 2 + 0.5);
        }
      } else {
        ctx.fillText(l.name, l.x + 5, l.y + 10);
        ctx.globalAlpha *= 0.7;
        ctx.fillText(l.size, l.x + 5, l.y + 24);
      }
      ctx.restore();
    }

    ctx.globalAlpha = 1;
    if (hoverRect) {
      const h = hoverRect as RectGeom;
      ctx.lineWidth = 2;
      ctx.strokeStyle = cssVar('--text', '#eee');
      ctx.strokeRect(h.x0 + 1, h.y0 + 1, h.x1 - h.x0 - 2, h.y1 - h.y0 - 2);
    }
    shown.current = next;
    if (p >= 1) tween.current = null;
    return p >= 1;
  }, [layout, width, height, hoverPath, stagedSet, fills]);

  useEffect(() => {
    let id = 0;
    const tick = (now: number) => { if (!draw(now)) id = requestAnimationFrame(tick); };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [draw]);

  const hit = (e: React.MouseEvent): Block | null => {
    const rect = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    return blockAt(layout, e.clientX - rect.left, e.clientY - rect.top);
  };

  const lastHover = useRef<string | null>(null);
  const leave = () => {
    lastHover.current = null;
    onHover(null);
  };

  // Same drag payload as list rows and the wheel, so the collector accepts it unchanged.
  const onDragStart = (e: React.DragEvent<HTMLCanvasElement>) => {
    const h = hit(e);
    if (!h || h.data.isRest || h.data.isHidden) {
      e.preventDefault();
      return;
    }
    const d = h.data;
    e.dataTransfer.setData('application/x-disko-item', JSON.stringify({ path: d.path, name: d.name, kind: d.kind, total: h.value ?? d.total, category: d.category, reason: d.reason }));
    e.dataTransfer.effectAllowed = 'copy';
  };

  return (
    <canvas
      ref={ref}
      role="img"
      aria-label="Disk usage blocks. Use the folder list to navigate, or drag a block to collect it."
      draggable
      style={{ width, height, display: 'block' }}
      onDragStart={onDragStart}
      onDragEnd={leave}
      onMouseMove={(e) => {
        const h = hit(e);
        const p = h ? h.data.path : null;
        if (lastHover.current !== p) {
          lastHover.current = p;
          onHover(p);
        }
        (e.currentTarget as HTMLCanvasElement).style.cursor = h && !h.data.isHidden ? 'pointer' : 'default';
      }}
      onMouseLeave={leave}
      onClick={(e) => {
        const h = hit(e);
        if (h && !h.data.isHidden) onClick(h);
      }}
      onContextMenu={(e) => {
        const h = hit(e);
        if (!h || h.data.isRest || h.data.isHidden) return;
        onContextMenu?.(h, e);
      }}
    />
  );
};

export const Treemap: React.FC<Props> = ({ datum, path, loading, hoverPath, colorBy, stagedPaths, onHover, onNavigate, onRemainder, onLayout, onContextMenu }) => {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });
  const light = useStore((s) => s.resolvedTheme === 'light');

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setBox({ width: Math.floor(width), height: Math.floor(height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(() => (datum && box.width > 0 && box.height > 0 ? layoutBlocks(datum, box.width, box.height) : null), [datum, box]);
  const held = useHeldLayout(layout, path);
  const shownLayout = held?.layout ?? null;

  useEffect(() => {
    if (!shownLayout) return;
    const m = new Map<string, string>();
    for (const d of shownLayout.nodes) m.set(d.data.path, nodeColor(d, colorBy, light));
    onLayout(m);
  }, [shownLayout, colorBy, light, onLayout]);

  return (
    <div ref={wrapRef} className="relative w-full h-full" style={{ pointerEvents: layout ? undefined : 'none' }}>
      {held && (
        <ZoomStage id={held.path} direction={held.direction} width={held.layout.width} height={held.layout.height}>
          <div className="rounded-lg overflow-hidden" style={{ width: held.layout.width, height: held.layout.height }}>
            <Blocks
              layout={held.layout}
              colorBy={colorBy}
              light={light}
              stagedPaths={stagedPaths}
              hoverPath={hoverPath}
              onHover={onHover}
              onContextMenu={(d, e) => onContextMenu?.({ ...d.data, total: d.value ?? d.data.total }, e)}
              onClick={(d) => {
                if (d.data.isRest) return onRemainder(d.parent?.data.path ?? d.data.path);
                if (d.data.kind === 'directory') return onNavigate(d.data.path);
                // A file opens the folder that holds it, unless that is the current folder.
                if (d.parent && d.parent.depth > 0) onNavigate(d.parent.data.path);
              }}
            />
          </div>
        </ZoomStage>
      )}
      {loading && (
        <div className="absolute top-2 right-2 pointer-events-none" style={{ zIndex: 2 }}>
          <Spinner size={14} />
        </div>
      )}
    </div>
  );
};
