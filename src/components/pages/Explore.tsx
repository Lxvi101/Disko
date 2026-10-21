import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Plus, Check, Eye, EyeOff, ChevronLeft, ChevronRight, Pin } from 'lucide-react';
import { useStore, type HiddenItem } from '../../store';
import { api } from '../../lib/api';
import { formatBytes, catMeta, formatDate, tagLabel, ACTION_LABELS, COST_LABELS } from '../../lib/format';
import { HIDDEN_COLOR, layoutTree, simplifyTree, folderForHover, withoutHidden, accountedDatum, isInside, RINGS } from '../../lib/sunburst';
import { restColor } from '../../lib/colors';
import { GREY } from '../../lib/colors';
import type { FileItem, TreeNode } from '../../types';
import { Sunburst } from '../viz/Sunburst';
import { Treemap } from '../viz/Treemap';
import { openExploreMenu, hideFromMap } from '../ContextMenu';

export const ExplorePage: React.FC = () => {
  const { currentPath, rootPath, currentItems, itemsLoading, navigate, navigateUp, staged, toggleStage, isStaged, advanced, colorBy, hoverPath, setHover, selectedPath, select, info, overview, resolvedTheme, dataVersion, viz, hiddenItems } = useStore();
  const [loadedTree, setTree] = useState<TreeNode | null>(null);
  // Navigation commits the path/items before its subtree request completes.
  // Never use the previous folder's tree for the new folder's legend or preview.
  const fullTree = loadedTree?.path === currentPath ? loadedTree : null;
  // Everything below works on the tree without the items the user hid.
  const hiddenSet = useMemo(() => new Set(hiddenItems.map((h) => h.path)), [hiddenItems]);
  const pruned = useMemo(() => (fullTree ? withoutHidden(fullTree, hiddenSet) : null), [fullTree, hiddenSet]);
  const tree = pruned?.tree ?? null;
  const removed = pruned?.removed ?? 0;
  const hiddenHere = useMemo(() => hiddenItems.filter((h) => isInside(h.path, currentPath)), [hiddenItems, currentPath]);
  const [treeLoading, setTreeLoading] = useState(false);
  const [colors, setColors] = useState<Map<string, string>>(new Map());
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [expandedRemainder, setExpandedRemainder] = useState<string | null>(null);

  const stagedPaths = useMemo(() => new Set(staged.map((s) => s.path)), [staged]);
  const rootLabel = rootPath === '/' ? 'Macintosh HD' : rootPath === info?.home ? 'Home' : rootPath.split('/').filter(Boolean).pop() ?? 'Root';

  useEffect(() => {
    if (!currentPath) return;
    setPreviewPath(null);
    let cancelled = false;
    setTreeLoading(true);
    api
      .subtree(currentPath, RINGS.length)
      .then((t) => !cancelled && setTree(t))
      .catch(() => {
        if (cancelled) return;
        const total = currentItems.reduce((a, b) => a + b.total, 0);
        setTree({
          path: currentPath,
          name: currentPath === rootPath ? rootLabel : currentPath.split('/').filter(Boolean).pop() ?? 'Home',
          kind: 'directory',
          total,
          category: 'review',
          reason: '',
          tag: 'user-data',
          action: 'quarantine',
          rest_total: 0,
          rest_count: 0,
          children: currentItems.filter((i) => i.total > 0).map((i) => ({ ...i, children: [], rest_total: 0, rest_count: 0 })),
        });
      })
      .finally(() => !cancelled && setTreeLoading(false));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPath, info?.db_path, dataVersion]);

  const onLayout = useCallback((m: Map<string, string>) => setColors(m), []);

  const displayTree = useMemo(() => tree && !advanced ? simplifyTree(tree) : tree, [tree, advanced]);

  // Inspect the full folder on chart hover; list-row hover only highlights its arc.
  const listParent = useMemo(() => tree ? folderForHover(tree, previewPath) : null, [tree, previewPath]);
  const onChartHover = useCallback((path: string | null) => {
    setHover(path);
    // Preserve the preview while moving from the chart into its list of contents.
    if (path) setPreviewPath(path);
  }, [setHover]);

  const volume = currentPath === '/' ? overview?.volume ?? info?.volume : undefined;
  const accounting = useMemo(() => displayTree ? layoutTree(displayTree, volume, removed) : null, [displayTree, volume, removed]);
  const blocks = useMemo(() => displayTree && viz === 'treemap' ? accountedDatum(displayTree, volume, removed).datum : null, [displayTree, volume, removed, viz]);

  const listItems: FileItem[] = useMemo(() => {
    if (listParent) {
      return listParent.children.map((c) => ({ path: c.path, name: c.name, kind: c.kind, total: c.total, allocated: 0, logical: 0, category: c.category, reason: c.reason, tag: c.tag, action: c.action, cost: 'unknown' }));
    }
    if (!advanced && displayTree) return displayTree.children.map(c => ({ ...c, allocated: 0, logical: 0, cost: 'unknown' }));
    return hiddenSet.size ? currentItems.filter((i) => !hiddenSet.has(i.path)) : currentItems;
  }, [listParent, currentItems, displayTree, advanced, hiddenSet]);

  const listTitle = listParent ? listParent.name : currentPath === rootPath ? rootLabel : currentPath.split('/').filter(Boolean).pop() ?? '';
  const listTotal = listParent ? listParent.total : accounting?.total ?? tree?.total ?? currentItems.reduce((a, b) => a + b.total, 0);

  const listTree = listParent ?? displayTree;
  const remainder = listTree?.rest_total ?? 0;
  const remainderExpanded = expandedRemainder === listTree?.path;
  const smallerItems = useMemo(() => {
    if (!remainderExpanded || !listTree || !tree) return [];
    const find = (n: TreeNode): TreeNode | undefined => n.path === listTree.path ? n : n.children.map(find).find(Boolean);
    const original = find(tree);
    const visible = new Set(listTree.children.map(c => c.path));
    return (original?.children ?? []).filter(c => !visible.has(c.path)).map(c => ({ ...c, allocated: 0, logical: 0, cost: 'unknown' }));
  }, [remainderExpanded, listTree, tree]);

  const onDragStart = useCallback((it: FileItem, e: React.DragEvent) => {
    e.dataTransfer.setData('application/x-disko-item', JSON.stringify(it));
    e.dataTransfer.effectAllowed = 'copy';
  }, []);
  const onOpen = useCallback((it: FileItem) => {
    if (it.kind === 'directory') navigate(it.path);
    else select(useStore.getState().selectedPath === it.path ? null : it.path);
  }, [navigate, select]);
  const onSelect = useCallback((it: FileItem) => select(useStore.getState().selectedPath === it.path ? null : it.path), [select]);
  const onStage = useCallback((it: FileItem) => toggleStage({ ...it, source: 'explore' }), [toggleStage]);

  const canGoUp = currentPath !== rootPath && currentPath.length > rootPath.length;

  // H hides whatever is under the pointer, in the map or the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { setPreviewPath(null); return; }
      if (e.key.toLowerCase() !== 'h' || e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      const path = useStore.getState().hoverPath;
      const find = (n: TreeNode): TreeNode | undefined => n.path === path ? n : n.children.map(find).find(Boolean);
      const it = (path && tree && path !== tree.path ? find(tree) : undefined) ?? listItems.find((i) => i.path === path);
      if (!it) return;
      e.preventDefault();
      hideFromMap(it);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [tree, listItems]);

  return (
    <div className={`h-full flex min-h-0 ${advanced ? '' : 'standard-explore'}`} onMouseLeave={() => { setPreviewPath(null); setHover(null); }}>
      <div className={`explore-wheel relative flex-1 min-w-0 min-h-0 p-6 pr-2 ${viz === 'treemap' ? 'blocks-view' : ''}`}>
        {viz === 'treemap' ? (
          <Treemap
            datum={blocks}
            path={tree?.path ?? null}
            loading={treeLoading || itemsLoading}
            hoverPath={hoverPath}
            colorBy={advanced ? colorBy : 'folder'}
            stagedPaths={stagedPaths}
            onHover={onChartHover}
            onNavigate={navigate}
            onRemainder={path => {
              if (advanced || path !== currentPath) navigate(path);
              if (!advanced) setExpandedRemainder(path);
            }}
            onLayout={onLayout}
            onContextMenu={(d, e) => openExploreMenu(e, d, colors.get(d.path))}
          />
        ) : (
        <Sunburst
          tree={tree}
          layout={accounting}
          loading={treeLoading || itemsLoading}
          canGoUp={canGoUp}
          hoverPath={hoverPath}
          colorBy={advanced ? colorBy : 'folder'}
          stagedPaths={stagedPaths}
          onHover={onChartHover}
          onNavigate={navigate}
          onRemainder={path => {
            if (advanced || path !== currentPath) navigate(path);
            if (!advanced) setExpandedRemainder(path);
          }}
          onUp={navigateUp}
          onLayout={onLayout}
          onContextMenu={(d, e) => openExploreMenu(e, d, colors.get(d.path))}
        />
        )}
        <HiddenTray items={hiddenHere} />
      </div>

      <div className="explore-legend w-[380px] flex-shrink-0 flex flex-col min-h-0 pr-6 pt-4 pb-4">
        {/* Hovering the chart previews a folder here; this is always the way back. Space is
            reserved even when hidden, so the list does not jump while the pointer moves. */}
        <button type="button" className="legend-back" style={{ visibility: listParent ? 'visible' : 'hidden' }} onClick={() => setPreviewPath(null)} tabIndex={listParent ? 0 : -1}>
          <ChevronLeft className="w-3.5 h-3.5" /> Back to {currentPath === rootPath ? rootLabel : currentPath.split('/').filter(Boolean).pop()}
        </button>
        <div className="legend-heading flex items-baseline justify-between px-2 pb-2">
          {/* The heading must commit with its rows and total, including rapid chart hovers. */}
          <button type="button" disabled={!listParent} onClick={() => setPreviewPath(null)} title={listParent ? 'Return to the current folder' : undefined} className="text-[13px] font-semibold truncate">
            {listTitle}
          </button>
          <span className="tnum text-[12px] flex-shrink-0 ml-3" style={{ color: 'var(--muted)' }}>
            {formatBytes(listTotal)}
          </span>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto">
          {listItems.map((it) => (
            <Row
              key={it.path}
              it={it}
              color={colors.get(it.path) ?? GREY}
              hovered={hoverPath === it.path}
              selected={selectedPath === it.path}
              staged={isStaged(it.path)}
              advanced={advanced}
              onHover={setHover}
              onOpen={onOpen}
              onSelect={onSelect}
              onStage={onStage}
              onDragStart={onDragStart}
            />
          ))}
          {!advanced && remainder > 0 && (
            <>
              <button className="w-full text-left rounded focus-visible:outline focus-visible:outline-1" aria-expanded={remainderExpanded} onClick={() => setExpandedRemainder(remainderExpanded ? null : listTree?.path ?? null)} title="Show or hide smaller items">
                <SpaceRow label={remainderExpanded ? 'smaller objects' : 'smaller objects…'} total={remainder} color={restColor(resolvedTheme === 'light')} />
              </button>
              {remainderExpanded && smallerItems.map(it => (
                <Row key={it.path} it={it} color={GREY} hovered={hoverPath === it.path} selected={selectedPath === it.path} staged={isStaged(it.path)} advanced={false} onHover={setHover} onOpen={onOpen} onSelect={onSelect} onStage={onStage} onDragStart={onDragStart} />
              ))}
              {remainderExpanded && smallerItems.length === 0 && <div className="px-5 py-2 text-[12px]" style={{ color: 'var(--muted)' }}>Turn on Details (⌘.) to list every item in this folder.</div>}
            </>
          )}
          {!listParent && (accounting?.hidden ?? 0) > 0 && (
            <SpaceRow label="unmeasured space…" total={accounting!.hidden} color={HIDDEN_COLOR} accent title="Used space the scan could not measure: unreadable folders, system files and filesystem data." />
          )}
          {!listParent && volume && (
            <div className="legend-space">
              <SpaceRow label="free space" total={volume.available} color="#bfc0c2" />
              <SpaceRow label="free + purgeable" total={volume.available_including_purgeable} approximate title="Space available including purgeable data. A dash means macOS has not reported this value." />
            </div>
          )}
          {listItems.length === 0 && !itemsLoading && (
            <div className="px-2 py-10 text-[12px]" style={{ color: 'var(--dim)' }}>
              Empty folder.
            </div>
          )}
        </div>
        <AnimatePresence>
          {advanced && selectedPath && (
            <Inspector path={selectedPath} items={currentItems} home={info?.home ?? ''} onClose={() => select(null)} />
          )}
        </AnimatePresence>
      </div>
    </div>
  );
};

/** Bottom-left pill for items hidden from this folder's map: bring back, or keep hidden next time. */
const HiddenTray: React.FC<{ items: HiddenItem[] }> = ({ items }) => {
  const { unhideItems, setHiddenPersist, info } = useStore();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const total = items.reduce((a, b) => a + b.total, 0);
  useEffect(() => { if (!items.length) setOpen(false); }, [items.length]);
  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => { if (e.target instanceof Node && !ref.current?.contains(e.target)) setOpen(false); };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false); } };
    window.addEventListener('mousedown', outside, true);
    window.addEventListener('keydown', key, true);
    return () => { window.removeEventListener('mousedown', outside, true); window.removeEventListener('keydown', key, true); };
  }, [open]);
  const home = info?.home ?? '';
  return (
    <div ref={ref} className="absolute left-6 bottom-3" style={{ zIndex: 5 }}>
      <AnimatePresence>
        {items.length > 0 && (
          <motion.button
            key="pill"
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 6 }}
            transition={{ duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
            onClick={() => setOpen(!open)}
            aria-expanded={open}
            className="hidden-pill flex items-center gap-1.5 h-7 px-2.5 rounded-full text-[12px]"
            style={{ background: open ? 'var(--bg-3)' : 'var(--bg-2)', border: '1px solid var(--line-2)', color: 'var(--muted)' }}
            title="Items hidden from the map"
          >
            <EyeOff className="w-3.5 h-3.5" />
            <span style={{ color: 'var(--text)' }}>{items.length} hidden</span>
            <span className="tnum">· {formatBytes(total)}</span>
          </motion.button>
        )}
      </AnimatePresence>
      <AnimatePresence>
        {open && items.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: 6, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 6, scale: 0.98 }}
            transition={{ duration: 0.14, ease: [0.2, 0.8, 0.2, 1] }}
            className="absolute left-0 bottom-9 w-[340px] panel-raised p-1.5"
            style={{ transformOrigin: 'bottom left' }}
          >
            <div className="max-h-[40vh] overflow-y-auto">
              {items.map((h) => (
                <div key={h.path} className="group flex items-center gap-2 pl-2 pr-1 h-8 rounded-md row-hover" title={h.path}>
                  <span className="flex-1 min-w-0">
                    <span className="block text-[12.5px] truncate">{h.name}</span>
                  </span>
                  <span className="tnum text-[11.5px] flex-shrink-0" style={{ color: 'var(--muted)' }}>{formatBytes(h.total)}</span>
                  <button
                    className="btn btn-ghost btn-icon !p-1"
                    aria-pressed={h.persist}
                    title={h.persist ? 'Stays hidden next time. Click to only hide it until Disko quits.' : 'Keep hidden next time'}
                    style={{ color: h.persist ? 'var(--text)' : 'var(--dim)' }}
                    onClick={() => setHiddenPersist(h.path, !h.persist)}
                  >
                    <Pin className="w-3.5 h-3.5" fill={h.persist ? 'currentColor' : 'none'} />
                  </button>
                  <button className="btn btn-ghost btn-icon !p-1" title={`Show ${h.path.startsWith(home) && home ? '~' + h.path.slice(home.length) : h.path} again`} onClick={() => unhideItems([h.path])}>
                    <Eye className="w-3.5 h-3.5" />
                  </button>
                </div>
              ))}
            </div>
            <div className="flex items-center justify-between gap-2 px-2 pt-1.5 mt-1" style={{ borderTop: '1px solid var(--line)' }}>
              <span className="text-[11px]" style={{ color: 'var(--dim)' }}>Pin to keep hidden next time</span>
              <button className="btn btn-ghost btn-sm" onClick={() => unhideItems(items.map((h) => h.path))}>Show all</button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};

const SpaceRow: React.FC<{ label: string; total?: number; color?: string; accent?: boolean; approximate?: boolean; title?: string }> = ({ label, total, color, accent, approximate, title }) => (
  <div className="space-row" title={title}>
    {approximate ? <span className="space-symbol">~</span> : <span className="space-dot" style={{ background: color }} />}
    <span className="space-label" style={accent ? { color: '#bf40d0' } : undefined}>{label}</span>
    <span className="tnum">{total == null ? '—' : formatBytes(total)}</span>
  </div>
);

const Row = React.memo<{
  it: FileItem;
  color: string;
  hovered: boolean;
  selected: boolean;
  staged: boolean;
  advanced: boolean;
  onHover: (p: string | null) => void;
  onOpen: (it: FileItem) => void;
  onSelect: (it: FileItem) => void;
  onStage: (it: FileItem) => void;
  onDragStart: (it: FileItem, e: React.DragEvent) => void;
}>(function Row({ it, color, hovered, selected, staged, advanced, onHover, onOpen, onSelect, onStage, onDragStart }) {
  const isDir = it.kind === 'directory';
  return (
    <div
      draggable
      onDragStart={(e) => onDragStart(it, e)}
      onMouseEnter={() => onHover(it.path)}
      onMouseLeave={() => onHover(null)}
      // Without the inspector a click has nothing to show, so it opens folders straight away.
      onClick={() => (!advanced && isDir ? onOpen(it) : onSelect(it))}
      onDoubleClick={() => onOpen(it)}
      onContextMenu={(e) => openExploreMenu(e, it, color)}
      className="legend-row group flex items-center gap-2.5 px-2 rounded-lg cursor-default"
      role="button"
      tabIndex={0}
      onKeyDown={e => {
        if (e.key === 'Enter') { e.preventDefault(); onOpen(it); }
        if (e.key === ' ') { e.preventDefault(); api.preview(it.path).catch(() => {}); }
      }}
      style={{ height: advanced ? 34 : 30, background: hovered || selected ? 'var(--bg-3)' : 'transparent', transition: 'background-color 90ms' }}
    >
      <span className="rounded-full flex-shrink-0" style={{ width: 8, height: 8, background: staged ? 'hsl(0 0% 62%)' : color }} />
      <span className={`flex-1 min-w-0 truncate text-[13px] ${staged ? 'line-through' : ''}`} style={{ color: staged ? 'var(--dim)' : isDir ? 'var(--text)' : 'var(--muted)' }}>
        {it.name}
      </span>
      {advanced && (
        <span className="text-[10.5px] flex-shrink-0 hidden group-hover:inline truncate max-w-[120px]" style={{ color: catMeta(it.category).text }}>
          {tagLabel(it.tag) || catMeta(it.category).label}
        </span>
      )}
      <span className="tnum text-[12.5px] flex-shrink-0" style={{ color: isDir ? 'var(--text)' : 'var(--muted)' }}>
        {formatBytes(it.total, 1, advanced)}
      </span>
      <span className="legend-actions flex items-center gap-0.5 flex-shrink-0 overflow-hidden transition-all" style={{ width: hovered || staged ? (isDir ? 74 : 50) : 0 }}>
        <button className="btn btn-ghost btn-icon !p-1" title="Reveal in Finder" onClick={(e) => { e.stopPropagation(); api.reveal(it.path).catch(() => {}); }}>
          <Eye className="w-3.5 h-3.5" />
        </button>
        <button
          className="btn btn-ghost btn-icon !p-1"
          title={it.category === 'protected' ? it.reason || 'Protected; cannot be collected' : staged ? 'Remove from collection' : 'Collect'}
          disabled={it.category === 'protected'}
          style={staged ? { color: 'var(--text)' } : undefined}
          onClick={(e) => { e.stopPropagation(); onStage(it); }}
        >
          {staged ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
        </button>
        {isDir && (
          <button className="btn btn-ghost btn-icon !p-1" title="Open" onClick={(e) => { e.stopPropagation(); onOpen(it); }}>
            <ChevronRight className="w-3.5 h-3.5" />
          </button>
        )}
      </span>
    </div>
  );
});

const Inspector: React.FC<{ path: string; items: FileItem[]; home: string; onClose: () => void }> = ({ path, items, home, onClose }) => {
  const [info, setInfo] = useState<FileItem | null>(null);
  useEffect(() => {
    let live = true;
    api.pathInfo(path).then((r) => live && setInfo(r)).catch(() => {});
    return () => { live = false; };
  }, [path]);
  const it = items.find((i) => i.path === path) ?? info;
  if (!it) return null;
  const m = catMeta(it.category);
  return (
    <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 8 }} className="panel p-3 mt-3 text-[12px]">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="font-semibold truncate text-[13px]">{it.name}</div>
          <div className="mono text-[10.5px] truncate selectable" style={{ color: 'var(--dim)' }}>{path.startsWith(home) ? '~' + path.slice(home.length) : path}</div>
        </div>
        <button className="btn btn-ghost btn-sm" onClick={onClose}>Close</button>
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-1 mt-2.5" style={{ color: 'var(--muted)' }}>
        <span>Size</span><span className="tnum text-right" style={{ color: 'var(--text)' }}>{formatBytes(it.total)}</span>
        {info?.logical ? (<><span>Logical</span><span className="tnum text-right" style={{ color: 'var(--text)' }}>{formatBytes(info.logical)}</span></>) : null}
        {info?.child_count != null && (<><span>Items inside</span><span className="tnum text-right" style={{ color: 'var(--text)' }}>{info.child_count}</span></>)}
        {info?.mtime_ns ? (<><span>Modified</span><span className="text-right" style={{ color: 'var(--text)' }}>{formatDate(Number(info.mtime_ns) / 1e9)}</span></>) : null}
        <span>Policy</span><span className="text-right" style={{ color: m.text }}>{m.label}{it.tag ? ` · ${tagLabel(it.tag)}` : ''}</span>
        {it.action && (<><span>Removal</span><span className="text-right" style={{ color: 'var(--text)' }}>{ACTION_LABELS[it.action] ?? it.action}</span></>)}
        {it.cost && it.cost !== 'unknown' && (<><span>Recreate</span><span className="text-right" style={{ color: 'var(--text)' }}>{COST_LABELS[it.cost] ?? it.cost}</span></>)}
      </div>
      <p className="mt-2 leading-snug" style={{ color: 'var(--muted)' }}>{it.reason}</p>
      {it.tool && (
        <div className="mono text-[10.5px] mt-1.5 px-2 py-1 rounded-md selectable truncate" style={{ background: 'var(--bg-3)', color: 'var(--text)' }} title={it.tool}>
          $ {it.tool}
        </div>
      )}
    </motion.div>
  );
};
