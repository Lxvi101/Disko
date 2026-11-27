import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy';
import { Check } from 'lucide-react';
import { useStore, sel } from '../../store';
import { itemProps } from '../../lib/interact';
import { splitBytes } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { catOf, tint } from '../../lib/categories';
import { Tip } from '../../components/ui/Tip';
import { Checkbox } from '../../components/ui/Checkbox';
import { hueFor, type MapNode } from './model';

type Tile = HierarchyRectangularNode<MapNode>;

/** Friendly treemap: rounded tinted tiles with gaps, single-hue light, icons, no tiny text. */
export const Blocks: React.FC<{ tree: MapNode }> = ({ tree }) => {
  const wrap = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 800, h: 600 });
  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setBox({ w: e.contentRect.width, h: e.contentRect.height }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const tiles = useMemo(() => {
    const own = (n: MapNode) => Math.max(0, n.bytes - (n.children?.reduce((a, c) => a + c.bytes, 0) ?? 0));
    const root = hierarchy<MapNode>(tree, (d) => d.children).sum(own).sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
    treemap<MapNode>()
      .tile(treemapSquarify.ratio(1.25))
      .size([box.w, box.h])
      .paddingInner(8)
      .paddingOuter(0)
      .paddingTop((d) => (d.depth === 1 && d.x1 - d.x0 > 190 && d.y1 - d.y0 > 130 ? 52 : d.depth === 1 ? 8 : 0))
      .paddingLeft((d) => (d.depth === 1 ? 8 : 0))
      .paddingRight((d) => (d.depth === 1 ? 8 : 0))
      .paddingBottom((d) => (d.depth === 1 ? 8 : 0))
      .round(true)(root);
    const top = (root.children ?? []) as Tile[];
    return top.map((t, i) => ({ t, hue: hueFor(i) }));
  }, [tree, box]);

  return (
    <div ref={wrap} className="relative w-full h-full">
      <AnimatePresence initial={false}>
        {tiles.map(({ t, hue }) => <Block key={t.data.path} t={t} hue={hue} depth={1} />)}
      </AnimatePresence>
    </div>
  );
};

const Block: React.FC<{ t: Tile; hue: number; depth: number; ox?: number; oy?: number }> = ({ t, hue, depth, ox = 0, oy = 0 }) => {
  const openFolder = useStore((s) => s.openFolder);
  const toggle = useStore((s) => s.toggle);
  const setHover = useStore((s) => s.setHover);
  const hovered = useStore((s) => s.hover === t.data.path);
  const picked = useStore((s) => s.selection.has(`p:${t.data.path}`));
  const dark = useStore((s) => s.dark);
  const n = t.data;
  const w = t.x1 - t.x0;
  const h = t.y1 - t.y0;
  if (w < 3 || h < 3) return null;
  const cat = catOf(n.clutter);
  // One tone per tile, laid over the field as a tint: clutter strong, folders a quiet slate.
  const c = cat ? cat.color : n.kind === 'hidden' ? '#7c7a92' : n.kind === 'other' ? '#6b7080' : `hsl(${hue} 38% ${dark ? 64 : 56}%)`;
  const a = cat ? (dark ? 40 : 42) : n.kind === 'hidden' ? 22 : n.kind === 'other' ? 12 : depth === 1 ? (dark ? 9 : 13) : dark ? 18 : 22;
  const vivid = !!cat || n.kind === 'hidden';
  const kids = (t.children ?? []) as Tile[];
  const headed = depth === 1 && w > 190 && h > 130 && kids.length > 0;
  const showLabel = !headed && w > 96 && h > 64;
  const showIcon = !headed && w > 44 && h > 44 && (!showLabel || h > 116);
  const s = splitBytes(t.value ?? n.bytes);
  const pickable = n.kind === 'dir' || n.kind === 'file';
  const selectable = pickable && !n.protected;
  const r = Math.min(18, Math.max(6, Math.min(w, h) * 0.12));
  return (
    <>
      <motion.div
        initial={{ opacity: 0, scale: 0.92 }}
        animate={{ opacity: 1, scale: 1, left: t.x0 - ox, top: t.y0 - oy, width: w, height: h }}
        exit={{ opacity: 0, scale: 0.9, transition: { duration: 0.2 } }}
        transition={{ type: 'spring', stiffness: 180, damping: 32 }}
        className="absolute group"
        style={{ borderRadius: r }}
        onMouseEnter={(e) => { e.stopPropagation(); setHover(n.path); }}
        onMouseLeave={() => setHover(null)}
      >
        <Tip content={n.kind === 'hidden' ? n.why : n.error} className="block w-full h-full">
          <div
            {...itemProps(pickable ? sel.node(n) : null, { folder: n.kind === 'dir' ? n.path : undefined })}
            onClick={(e) => {
              e.stopPropagation();
              if (!pickable) return;
              if (n.kind === 'dir' && !e.metaKey && !e.altKey) openFolder(n.path);
              else if (selectable) toggle(sel.node(n));
            }}
            className="relative w-full h-full overflow-hidden"
            style={{
              borderRadius: r,
              background: `linear-gradient(180deg, ${tint(c, a + 7)}, ${tint(c, a)} 70%)`,
              boxShadow: `inset 0 0 0 1px ${tint(c, hovered ? 70 : vivid ? 34 : 22)}, inset 0 1px 0 var(--hi)${picked ? ', 0 0 0 3px var(--accent)' : ''}${depth === 1 ? `, 0 18px 36px -30px rgba(0,0,0,${dark ? 0.7 : 0.25})` : ''}`,
              filter: hovered ? 'brightness(1.08)' : undefined,
              transition: 'filter 200ms ease, box-shadow 200ms ease',
              cursor: n.kind === 'dir' ? 'pointer' : 'default',
              color: 'var(--text)',
            }}
          >
            {picked && <div className="absolute inset-0" style={{ background: 'var(--accent-soft)' }} />}
            {headed && (
              <div className="absolute left-0 right-0 top-0 h-[52px] flex items-center gap-2.5 px-3.5">
                <NodeIcon node={n} size={28} />
                <span className="font-medium text-[15px] truncate min-w-0">{n.name}</span>
                <span className="flex-1" />
                <span className="num font-semibold text-[16px] whitespace-nowrap shrink-0">{s.value} <span className="unit">{s.unit}</span></span>
                <span className={`shrink-0 ${picked ? '' : 'w-0 overflow-hidden group-hover:w-auto group-hover:overflow-visible'}`}>
                  <Checkbox checked={picked} onChange={() => toggle(sel.node(n))} color="var(--accent)" size={22} />
                </span>
              </div>
            )}
            {(showIcon || showLabel) && (
              <div className="absolute inset-0 flex flex-col items-center justify-center text-center p-2 gap-1">
                {showIcon && <NodeIcon node={n} size={Math.min(40, Math.max(22, Math.min(w, h) * 0.26))} />}
                {showLabel && <span className="font-medium truncate max-w-full text-[14px]">{n.name}</span>}
                {showLabel && <span className="num font-semibold text-[15px]">{s.value} <span className="unit">{s.unit}</span></span>}
              </div>
            )}
          </div>
        </Tip>
        {selectable && !headed && w > 60 && h > 50 && (
          <button
            aria-label={picked ? 'Deselect' : 'Select'}
            onClick={(e) => { e.stopPropagation(); toggle(sel.node(n)); }}
            className={`absolute flex items-center justify-center rounded-full transition-opacity ${picked ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
            style={{ right: 8, top: 8, width: 24, height: 24, background: picked ? 'var(--accent)' : 'var(--solid)', border: picked ? 'none' : '1px solid var(--line-2)', color: picked ? 'white' : 'var(--muted)' }}
          >
            <Check size={14} strokeWidth={3} />
          </button>
        )}
        {headed && (
          <div className="absolute inset-0 pointer-events-none">
            <div className="relative w-full h-full pointer-events-auto" style={{ pointerEvents: 'none' }}>
              {kids.map((k) => (
                <div key={k.data.path} style={{ pointerEvents: 'auto' }}>
                  <Block t={k} hue={hue} depth={2} ox={t.x0} oy={t.y0} />
                </div>
              ))}
            </div>
          </div>
        )}
      </motion.div>
    </>
  );
};
