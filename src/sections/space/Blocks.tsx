import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy';
import { Check } from 'lucide-react';
import { useStore, sel } from '../../store';
import { splitBytes } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { catOf } from '../../lib/categories';
import { Tip } from '../../components/ui/Tip';
import { Checkbox } from '../../components/ui/Checkbox';
import { hueFor, type MapNode } from './model';

type Tile = HierarchyRectangularNode<MapNode>;

/** Friendly treemap: rounded tiles with gaps, soft gradients, icons, no tiny text. */
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
  const [c1, c2] = cat
    ? [cat.color, cat.color2]
    : n.kind === 'hidden' ? ['#7d6fb3', '#584b8f']
    : n.kind === 'other' ? (dark ? ['#3b4057', '#2c3044'] : ['#cfd3e3', '#bcc1d6'])
    : dark ? [`hsl(${hue} 42% ${depth === 1 ? 34 : 44}%)`, `hsl(${hue + 18} 46% ${depth === 1 ? 24 : 34}%)`] : [`hsl(${hue} 70% ${depth === 1 ? 86 : 76}%)`, `hsl(${hue + 18} 64% ${depth === 1 ? 78 : 68}%)`];
  const vivid = !!cat || n.kind === 'hidden';
  const textColor = vivid || dark ? 'white' : '#1b1d3a';
  const kids = (t.children ?? []) as Tile[];
  const headed = depth === 1 && w > 190 && h > 130 && kids.length > 0;
  const showLabel = !headed && w > 96 && h > 64;
  const showIcon = !headed && w > 44 && h > 44 && (!showLabel || h > 116);
  const s = splitBytes(t.value ?? n.bytes);
  const selectable = n.kind === 'dir' || n.kind === 'file';
  const r = Math.min(18, Math.max(6, Math.min(w, h) * 0.12));
  return (
    <>
      <motion.div
        initial={{ opacity: 0, scale: 0.92 }}
        animate={{ opacity: 1, scale: 1, left: t.x0 - ox, top: t.y0 - oy, width: w, height: h }}
        exit={{ opacity: 0, scale: 0.9, transition: { duration: 0.2 } }}
        transition={{ type: 'spring', stiffness: 220, damping: 30 }}
        className="absolute group"
        style={{ borderRadius: r }}
        onMouseEnter={(e) => { e.stopPropagation(); setHover(n.path); }}
        onMouseLeave={() => setHover(null)}
      >
        <Tip content={n.kind === 'hidden' ? n.why : n.error} className="block w-full h-full">
          <div
            onClick={(e) => {
              e.stopPropagation();
              if (!selectable) return;
              if (n.kind === 'dir' && !e.metaKey && !e.altKey) openFolder(n.path);
              else toggle(sel.node(n));
            }}
            className="relative w-full h-full overflow-hidden"
            style={{
              borderRadius: r,
              background: `linear-gradient(155deg, ${c1}, ${c2})`,
              opacity: vivid || depth === 1 ? 1 : 0.95,
              boxShadow: `inset 0 1px 0 rgba(255,255,255,${dark ? 0.14 : 0.5}), inset 0 0 0 1px rgba(255,255,255,${hovered ? 0.5 : 0.06})${picked ? ', 0 0 0 3px var(--accent)' : ''}${depth === 1 ? `, 0 14px 30px -18px ${vivid ? c1 : 'rgba(0,0,0,.6)'}` : ''}`,
              filter: hovered ? 'brightness(1.12)' : undefined,
              transition: 'filter 140ms ease, box-shadow 140ms ease',
              cursor: n.kind === 'dir' ? 'pointer' : 'default',
              color: textColor,
            }}
          >
            {picked && <div className="absolute inset-0" style={{ background: 'rgba(255,255,255,.25)' }} />}
            {headed && (
              <div className="absolute left-0 right-0 top-0 h-[52px] flex items-center gap-2.5 px-3.5">
                <NodeIcon node={n} onColor={vivid} size={28} />
                <span className="font-semibold text-[15px] truncate min-w-0">{n.name}</span>
                <span className="flex-1" />
                <span className="num font-bold text-[16px] opacity-90 whitespace-nowrap shrink-0">{s.value} {s.unit}</span>
                <span className={`shrink-0 ${picked ? '' : 'w-0 overflow-hidden group-hover:w-auto group-hover:overflow-visible'}`}>
                  <Checkbox checked={picked} onChange={() => toggle(sel.node(n))} color="var(--accent)" size={22} />
                </span>
              </div>
            )}
            {(showIcon || showLabel) && (
              <div className="absolute inset-0 flex flex-col items-center justify-center text-center p-2 gap-1">
                {showIcon && <NodeIcon node={n} onColor={vivid} size={Math.min(40, Math.max(22, Math.min(w, h) * 0.26))} />}
                {showLabel && <span className="font-semibold truncate max-w-full text-[14px]">{n.name}</span>}
                {showLabel && <span className="num font-bold text-[15px] opacity-85">{s.value} {s.unit}</span>}
              </div>
            )}
          </div>
        </Tip>
        {selectable && !headed && w > 60 && h > 50 && (
          <button
            aria-label={picked ? 'Deselect' : 'Select'}
            onClick={(e) => { e.stopPropagation(); toggle(sel.node(n)); }}
            className={`absolute flex items-center justify-center rounded-full transition-opacity ${picked ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
            style={{ right: 8, top: 8, width: 24, height: 24, background: picked ? 'var(--accent)' : 'rgba(0,0,0,.35)', color: 'white' }}
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
