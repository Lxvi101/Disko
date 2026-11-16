import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { hierarchy, pack } from 'd3-hierarchy';
import { Check } from 'lucide-react';
import { useStore, sel } from '../../store';
import { splitBytes } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { catOf, tint } from '../../lib/categories';
import { Tip } from '../../components/ui/Tip';
import { hueFor, type MapNode } from './model';

/** The current folder's items as frosted glass bubbles sized by space. */
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
  // One tone per bubble; clutter carries its category tone, folders a quiet slate.
  const base = cat?.color ?? (node.kind === 'hidden' ? '#7c7a92' : node.kind === 'other' ? '#6b7080' : hsl(hue, 40, dark ? 64 : 58));
  const a = cat ? (dark ? 32 : 36) : dark ? 17 : 20;
  const s = splitBytes(node.bytes);
  const selectable = node.kind === 'dir' || node.kind === 'file';
  const icon = Math.min(64, Math.max(16, r * 0.42));
  return (
    <motion.div
      layout={false}
      initial={{ opacity: 0, scale: 0.4, left: x - r, top: y - r, width: r * 2, height: r * 2 }}
      animate={{ opacity: 1, scale: 1, left: x - r, top: y - r, width: r * 2, height: r * 2 }}
      exit={{ opacity: 0, scale: 0.2, transition: { duration: 0.25 } }}
      transition={{ type: 'spring', stiffness: 140, damping: 26, delay: Math.min(0.25, index * 0.012) }}
      className="absolute group"
      onMouseEnter={() => setHover(node.path)}
      onMouseLeave={() => setHover(null)}
    >
      <Tip content={node.kind === 'hidden' ? node.why : node.error} className="block w-full h-full">
        <button
          className="relative w-full h-full rounded-full flex flex-col items-center justify-center text-center overflow-hidden"
          style={{
            background: `radial-gradient(120% 95% at 50% 0%, ${tint(base, a + 10)}, ${tint(base, a)} 55%, ${tint(base, Math.max(6, a - 6))})`,
            boxShadow: `inset 0 0 0 1px ${tint(base, hovered || picked ? 75 : 34)}, inset 0 1px 0 var(--hi), inset 0 -${r * 0.25}px ${r * 0.45}px -${r * 0.3}px ${tint(base, a + 16)}, 0 ${r * 0.14}px ${r * 0.5}px -${r * 0.3}px ${tint(base, 55)}${picked ? `, 0 0 0 3px var(--accent)` : ''}`,
            transform: hovered ? 'scale(1.02)' : undefined,
            transition: 'transform 220ms ease, box-shadow 220ms ease',
            cursor: node.kind === 'dir' ? 'pointer' : 'default',
          }}
          onClick={(e) => {
            if (!selectable) return;
            if (node.kind === 'dir' && !e.metaKey && !e.altKey) openFolder(node.path);
            else toggle(sel.node(node));
          }}
        >
          <span className="relative">
            {r > 16 && <NodeIcon node={node} size={icon} />}
          </span>
          {r > 46 && (
            <span className="relative font-medium truncate px-3 mt-1.5" style={{ maxWidth: r * 1.6, fontSize: Math.min(16, Math.max(12, r * 0.15)), color: 'var(--text)' }}>
              {node.name}
            </span>
          )}
          {r > 32 && (
            <span className="relative num font-semibold" style={{ fontSize: Math.min(22, Math.max(12, r * 0.17)), color: 'var(--text)', opacity: 0.86 }}>
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
