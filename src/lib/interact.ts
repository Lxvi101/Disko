// Drag and right-click for anything removable, the same everywhere: map arcs, bubbles, blocks,
// list rows, clutter, apps, sessions. Dragging an item that's part of the selection drags the
// whole selection, like Finder.
import type React from 'react';
import { useStore, type SelItem } from '../store';
import { bytes } from './format';

let payload: SelItem[] = [];

/** What the current drag carries (readable during dragover, unlike dataTransfer). */
export const dragged = () => payload;

export function startDrag(e: React.DragEvent, item: SelItem) {
  const st = useStore.getState();
  const items = st.selection.has(item.key) ? [...st.selection.values()] : [item];
  payload = items;
  st.closeMenu();
  e.dataTransfer.setData('application/x-disko', items.map((i) => i.key).join('\n'));
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setDragImage(ghost(items), 18, 18);
  // Let the browser grab the drag image before the dock re-renders the page.
  requestAnimationFrame(() => st.setDragging(items));
}

export function endDrag() {
  payload = [];
  // The pointer left during the drag without any mouseleave; don't leave a highlight behind.
  useStore.getState().setHover(null);
  useStore.getState().setDragging(null);
}

/** A compact chip under the pointer: name (or count) and size. */
function ghost(items: SelItem[]): HTMLElement {
  const css = (n: string, f: string) => getComputedStyle(document.documentElement).getPropertyValue(n).trim() || f;
  const el = document.createElement('div');
  el.style.cssText = `position:fixed;top:-1000px;left:-1000px;display:flex;align-items:center;gap:10px;padding:8px 14px;border-radius:14px;font:600 14px -apple-system,system-ui,sans-serif;white-space:nowrap;pointer-events:none;background:${css('--solid', '#16171c')};color:${css('--text', '#eee')};border:1px solid ${css('--line-2', '#393a41')};box-shadow:0 12px 30px -12px rgba(0,0,0,.5)`;
  const name = document.createElement('span');
  name.textContent = items.length === 1 ? items[0].title : `${items.length} items`;
  name.style.cssText = 'max-width:260px;overflow:hidden;text-overflow:ellipsis';
  const size = document.createElement('span');
  size.textContent = bytes(items.reduce((a, i) => a + i.bytes, 0));
  size.style.cssText = `color:${css('--muted', '#999')};font-variant-numeric:tabular-nums;font-weight:500`;
  el.append(name, size);
  document.body.appendChild(el);
  requestAnimationFrame(() => el.remove());
  return el;
}

export function openMenu(e: React.MouseEvent, item: SelItem, opts: { folder?: string; path?: string } = {}) {
  e.preventDefault();
  e.stopPropagation();
  const st = useStore.getState();
  // Right-clicking part of the selection acts on all of it.
  const items = st.selection.has(item.key) ? [...st.selection.values()] : [item];
  st.openMenu({ x: e.clientX, y: e.clientY, items, folder: opts.folder, path: opts.path ?? item.paths[0] });
}

/** Spread on any element that stands for one removable item. */
export function itemProps(item: SelItem | null, opts: { folder?: string; path?: string } = {}) {
  if (!item) return {};
  return {
    draggable: !item.locked,
    onDragStart: (e: React.DragEvent) => { e.stopPropagation(); startDrag(e, item); },
    onDragEnd: endDrag,
    onContextMenu: (e: React.MouseEvent) => openMenu(e, item, opts),
  };
}
