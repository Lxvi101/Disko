import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, pack } from 'd3-hierarchy';
import { Check } from 'lucide-react';
import { useStore, sel } from '../../store';
import { splitBytes } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { catOf } from '../../lib/categories';
import { Tip } from '../../components/ui/Tip';
import { hueFor, type MapNode } from './model';

/** Space Lens style: the current folder's items as glossy bubbles sized by space. */
export const Bubbles: React.FC<{ tree: MapNode }> = ({ tree }) => {
  const wrap = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 800, h: 600 });
  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setBox({ w: e.contentRect.width, h: e.contentRect.height }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const circles = useMemo(() => {
    const kids = (tree.children ?? []).filter((c) => c.bytes > 0);
    const root = hierarchy<MapNode>({ ...tree, children: kids }, (d) => (d === tree || d.path === tree.path ? d.children : undefined))
      .sum((d) => (d.path === tree.path ? 0 : d.bytes))
      .sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
    const d = Math.min(box.w, box.h);
    pack<MapNode>().size([d, d]).padding(Math.max(6, d * 0.012))(root);
    const ox = (box.w - d) / 2;
    const oy = (box.h - d) / 2;
    return (root.children ?? []).map((c) => {
      const p = c as unknown as { x: number; y: number; r: number; data: MapNode };
      return { node: p.data, x: p.x + ox, y: p.y + oy, r: p.r };
    });
  }, [tree, box]);

  return (
    <div ref={wrap} className="relative w-full h-full">
      <AnimatePresence initial={false}>
        {circles.map((c, i) => <Bubble key={c.node.path} {...c} index={i} hue={hueFor(i)} />)}
      </AnimatePresence>
    </div>
  );
};

const Bubble: React.FC<{ node: MapNode; x: number; y: number; r: number; index: number; hue: number }> = ({ node, x, y, r, index, hue }) => {
  const openFolder = useStore((s) => s.openFolder);
  const toggle = useStore((s) => s.toggle);
  const setHover = useStore((s) => s.setHover);
  const hovered = useStore((s) => s.hover === node.path);
  const picked = useStore((s) => s.selection.has(`p:${node.path}`));
  const dark = useStore((s) => s.dark);
  const cat = catOf(node.clutter);
  const base = cat?.color ?? (node.kind === 'hidden' ? '#7d6fb3' : node.kind === 'other' ? '#6b7290' : hsl(hue, 72, dark ? 66 : 64));
  const base2 = cat?.color2 ?? (node.kind === 'hidden' ? '#4b3f7a' : node.kind === 'other' ? '#444a63' : hsl(hue + 16, 70, dark ? 46 : 52));
  const s = splitBytes(node.bytes);
  const selectable = node.kind === 'dir' || node.kind === 'file';
  const icon = Math.min(64, Math.max(16, r * 0.42));
  return (
    <motion.div
      layout={false}
      initial={{ opacity: 0, scale: 0.4, left: x - r, top: y - r, width: r * 2, height: r * 2 }}
      animate={{ opacity: 1, scale: 1, left: x - r, top: y - r, width: r * 2, height: r * 2 }}
      exit={{ opacity: 0, scale: 0.2, transition: { duration: 0.25 } }}
      transition={{ type: 'spring', stiffness: 170, damping: 24, delay: Math.min(0.25, index * 0.012) }}
      className="absolute group"
      onMouseEnter={() => setHover(node.path)}
      onMouseLeave={() => setHover(null)}
    >
      <Tip content={node.kind === 'hidden' ? node.why : node.error} className="block w-full h-full">
        <button
          className="relative w-full h-full rounded-full flex flex-col items-center justify-center text-center overflow-hidden"
          style={{
            background: `radial-gradient(circle at 50% 115%, ${base}cc 0%, ${base2}66 45%, ${base2}1f 75%), radial-gradient(circle at 50% 50%, ${base}14, ${base}30)`,
            boxShadow: `inset 0 0 0 1.5px ${base}${hovered || picked ? 'ee' : '66'}, inset 0 ${r * 0.08}px ${r * 0.3}px rgba(255,255,255,.12), 0 ${r * 0.1}px ${r * 0.5}px -${r * 0.2}px ${base}aa${picked ? `, 0 0 0 4px var(--accent)` : ''}`,
            transform: hovered ? 'scale(1.03)' : undefined,
            transition: 'transform 160ms ease, box-shadow 160ms ease',
            cursor: node.kind === 'dir' ? 'pointer' : 'default',
          }}
          onClick={(e) => {
            if (!selectable) return;
            if (node.kind === 'dir' && !e.metaKey && !e.altKey) openFolder(node.path);
            else toggle(sel.node(node));
          }}
        >
          {/* glass highlight */}
          <span className="absolute rounded-full pointer-events-none" style={{ left: '20%', right: '20%', top: '3%', height: '20%', background: 'radial-gradient(ellipse at 50% 0%, rgba(255,255,255,.38), rgba(255,255,255,0) 70%)' }} />
          <span className="relative" style={{ filter: 'drop-shadow(0 4px 10px rgba(0,0,0,.3))' }}>
            {r > 16 && <NodeIcon node={node} size={icon} />}
          </span>
          {r > 46 && (
            <span className="relative font-semibold truncate px-3 mt-1.5" style={{ maxWidth: r * 1.6, fontSize: Math.min(16, Math.max(12, r * 0.15)), color: dark ? 'white' : '#1b1640' }}>
              {node.name}
            </span>
          )}
          {r > 32 && (
            <span className="relative num font-bold" style={{ fontSize: Math.min(22, Math.max(12, r * 0.17)), color: dark ? 'rgba(255,255,255,.85)' : 'rgba(27,22,64,.75)' }}>
              {s.value} {s.unit}
            </span>
          )}
        </button>
      </Tip>
      {selectable && r > 22 && (
        <button
          aria-label={picked ? 'Deselect' : 'Select'}
          onClick={() => toggle(sel.node(node))}
          className={`absolute flex items-center justify-center rounded-full transition-opacity ${picked ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
          style={{ right: r * 0.29 - 13, top: r * 0.29 - 13, width: 26, height: 26, background: picked ? 'var(--accent)' : 'var(--solid)', border: '1.5px solid var(--line-2)', color: picked ? 'white' : 'var(--muted)' }}
        >
          <Check size={15} strokeWidth={3} />
        </button>
      )}
    </motion.div>
  );
};

/** Hex so alpha suffixes work. */
function hsl(h: number, s: number, l: number): string {
  s /= 100; l /= 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => Math.round(255 * (l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1))))).toString(16).padStart(2, '0');
  return `#${f(0)}${f(8)}${f(4)}`;
}
