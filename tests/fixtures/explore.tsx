import React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { ExplorePage } from '../../src/components/pages/Explore';
import { useStore } from '../../src/store';
import { api } from '../../src/lib/api';
import { layoutTree, simplifyTree, START_ANGLE } from '../../src/lib/sunburst';
import type { TreeNode } from '../../src/types';
import '../../src/index.css';

const node = (path: string, total: number, children: TreeNode[] = []): TreeNode => ({
  path, name: path.split('/').pop() || 'Disk', total, children, kind: 'directory',
  category: 'review', reason: '', tag: '', action: 'quarantine', rest_total: 0, rest_count: 0,
});
const apps = node('/Applications', 106e9, [node('/Applications/Topaz Photo.app', 24e9), node('/Applications/Topaz Video.app', 11e9)]);
const updates = node('/Updates', 4e9, [node('/Updates/Installer', 4e9)]);
const disk = node('/', 110e9, [apps, updates]);
const folders = new Map([disk, apps, updates].map(n => [n.path, n]));
const pending = new Map<string, (tree: TreeNode) => void>();
let delay = false;
api.children = async path => folders.get(path)!.children.map(n => ({ ...n, allocated: 0, logical: 0, cost: 'unknown' }));
api.subtree = path => delay
  ? new Promise(resolve => pending.set(path, resolve))
  : Promise.resolve(folders.get(path)!);
useStore.setState({ rootPath: '/', currentPath: '/', currentItems: await api.children('/'), advanced: false });
document.documentElement.dataset.theme = 'light';
document.body.innerHTML = '<div id="fixture" style="width:1200px;height:800px"></div>';
createRoot(document.getElementById('fixture')!).render(<ExplorePage />);

const snapshot = () => ({
  title: document.querySelector('.legend-heading button')?.textContent?.trim(),
  rows: Array.from(document.querySelectorAll('.legend-row')).map(el => el.textContent),
  total: document.querySelector('.legend-heading > span')?.textContent,
});
Object.assign(window, { exploreTest: {
  snapshot,
  async hover(path: string) {
    const canvas = document.querySelector('canvas')!;
    const rect = canvas.getBoundingClientRect();
    const layout = layoutTree(useStore.getState().advanced ? disk : simplifyTree(disk));
    const arc = layout.byPath.get(path)!;
    const angle = (arc.x0 + arc.x1) / 2 + START_ANGLE - Math.PI / 2;
    const radius = (rect.width / 2 - 6) * 0.25;
    flushSync(() => canvas.dispatchEvent(new MouseEvent('mousemove', {
      bubbles: true, clientX: rect.left + rect.width / 2 + Math.cos(angle) * radius,
      clientY: rect.top + rect.height / 2 + Math.sin(angle) * radius,
    })));
    // Continuous mouse events commit at frame boundaries in React.
    await new Promise(requestAnimationFrame);
    return snapshot();
  },
  advanced(value: boolean) { flushSync(() => useStore.setState({ advanced: value })); },
  async navigate(path: string) { delay = true; await useStore.getState().navigate(path); },
  resolve(path: string) { pending.get(path)!(folders.get(path)!); },
} });
