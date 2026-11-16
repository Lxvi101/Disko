import React, { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronRight } from 'lucide-react';
import { useStore, sel } from '../../store';
import { age, splitBytes, count } from '../../lib/format';
import { NodeIcon } from '../../lib/icons';
import { catOf } from '../../lib/categories';
import { api } from '../../lib/api';
import { Checkbox } from '../../components/ui/Checkbox';
import { Tip } from '../../components/ui/Tip';
import type { TreeNode } from '../../lib/types';
import type { MapNode } from './model';

/** Left panel: the current folder's items, biggest first. Clicking a folder opens it. */
export const FolderList: React.FC<{ tree: MapNode }> = ({ tree }) => {
  const kids = tree.children ?? [];
  const max = Math.max(1, ...kids.map((k) => k.bytes));
  const s = splitBytes(tree.bytes);
  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-4 px-5 pt-5 pb-4">
        <NodeIcon node={{ ...tree, kind: tree.path === '/' ? 'disk' : 'dir' }} size={52} />
        <div className="min-w-0">
          <div className="text-[20px] font-bold truncate">{tree.name}</div>
          <div className="flex items-baseline gap-1.5">
            <span className="num text-[22px] font-bold">{s.value}</span>
            <span className="num text-[14px] font-semibold muted">{s.unit}</span>
            <span className="muted text-[14px] ml-1">· {count(tree.files)} files</span>
          </div>
        </div>
      </div>
      <div className="scroll flex-1 min-h-0 px-2 pb-24">
        <AnimatePresence initial={false}>
          {kids.map((k, i) => <Row key={k.path} node={k} max={max} index={i} />)}
        </AnimatePresence>
      </div>
    </div>
  );
};

export const Row: React.FC<{ node: MapNode; max: number; index: number; indent?: number; expandable?: boolean }> = ({ node, max, index, indent = 0, expandable }) => {
  const openFolder = useStore((s) => s.openFolder);
  const toggle = useStore((s) => s.toggle);
  const setHover = useStore((s) => s.setHover);
  const hovered = useStore((s) => s.hover === node.path);
  const picked = useStore((s) => s.selection.has(`p:${node.path}`));
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const cat = catOf(node.clutter);
  const s = splitBytes(node.bytes);
  const selectable = node.kind === 'dir' || node.kind === 'file';
  const color = cat?.color ?? (node.kind === 'hidden' ? '#8f82c4' : 'var(--accent)');

  // Hovering the map scrolls the matching row into view.
  useEffect(() => {
    if (hovered && ref.current && !ref.current.matches(':hover')) ref.current.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [hovered]);

  const click = () => {
    if (expandable && node.kind === 'dir') return setOpen((o) => !o);
    if (node.kind === 'dir') openFolder(node.path);
    else if (selectable) toggle(sel.node(node));
  };

  return (
    <motion.div layout="position" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, x: -30, height: 0 }} transition={{ duration: 0.22, delay: Math.min(0.2, index * 0.015) }}>
      <Tip content={node.kind === 'hidden' ? node.why : node.error} className="block">
        <div
          ref={ref}
          onClick={click}
          onDoubleClick={() => expandable && node.kind === 'dir' && openFolder(node.path)}
          onMouseEnter={() => setHover(node.path)}
          onMouseLeave={() => setHover(null)}
          className="flex items-center gap-3 rounded-2xl pr-3 py-2.5 transition-colors"
          style={{ paddingLeft: 10 + indent * 26, background: hovered ? 'var(--panel-2)' : picked ? 'color-mix(in srgb, var(--accent) 10%, transparent)' : undefined, cursor: node.kind === 'dir' ? 'pointer' : 'default' }}
        >
          {selectable ? <Checkbox checked={picked} onChange={() => toggle(sel.node(node))} /> : <span className="w-[22px] shrink-0" />}
          {expandable && (
            <ChevronRight size={16} className="muted shrink-0 transition-transform" style={{ transform: open ? 'rotate(90deg)' : undefined, opacity: node.kind === 'dir' ? 1 : 0 }} />
          )}
          <NodeIcon node={node} size={34} />
          <div className="flex-1 min-w-0">
            <div className="flex items-baseline gap-2">
              <span className="font-semibold text-[15px] truncate">{node.name}</span>
            </div>
            <div className="h-[5px] mt-1.5 bar-track">
              <motion.div className="h-full rounded-full" style={{ background: cat ? `linear-gradient(90deg, ${cat.color}, ${cat.color2})` : color, opacity: cat ? 1 : 0.55 }} animate={{ width: `${Math.max(1.5, (node.bytes / max) * 100)}%` }} transition={{ type: 'spring', stiffness: 140, damping: 24 }} />
            </div>
          </div>
          {expandable && <span className="muted text-[13px] w-[90px] text-right shrink-0">{age(node.newest)}</span>}
          <div className="text-right shrink-0 w-[84px]">
            <span className="num font-bold text-[16px]">{s.value}</span>
            <span className="num font-semibold text-[13px] muted ml-1">{s.unit}</span>
          </div>
        </div>
      </Tip>
      {expandable && open && <Children path={node.path} indent={indent + 1} />}
    </motion.div>
  );
};

/** Lazily loads one level for the list view's tree-table. */
const Children: React.FC<{ path: string; indent: number }> = ({ path, indent }) => {
  const version = useStore((s) => s.treeVersion + s.scanTick);
  const gone = useStore((s) => s.gone);
  const [t, setT] = useState<TreeNode | null>(null);
  useEffect(() => { api.tree(path, 1).then(setT).catch(() => {}); }, [path, version]);
  if (!t) return <div className="h-10 mx-10 my-1 rounded-xl shimmer" />;
  const kids = (t.children ?? []).filter((c) => !gone.has(c.path));
  const max = Math.max(1, ...kids.map((k) => k.bytes));
  return <>{kids.map((k, i) => <Row key={k.path} node={k} max={max} index={i} indent={indent} expandable />)}</>;
};

export const ListView: React.FC<{ tree: MapNode }> = ({ tree }) => {
  const kids = tree.children ?? [];
  const max = Math.max(1, ...kids.map((k) => k.bytes));
  return (
    <div className="scroll h-full px-3 pb-28">
      <AnimatePresence initial={false}>
        {kids.map((k, i) => <Row key={k.path} node={k} max={max} index={i} expandable />)}
      </AnimatePresence>
    </div>
  );
};
