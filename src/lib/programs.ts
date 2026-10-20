import type { JunkItem, QuickGroup, QuickItem } from '../types';
import { describeJunk } from './junk';

/** The programs that own reclaimable space. Everything Disko offers is clustered under one of these. */
export interface Program { id: string; name: string; emoji: string; color: string; blurb: string }

export const PROGRAMS: Program[] = [
  { id: 'Rust', name: 'Rust', emoji: '🦀', color: '#e8784a', blurb: 'Cargo build folders and crate downloads' },
  { id: 'Node.js', name: 'Node.js', emoji: '📦', color: '#6cc070', blurb: 'node_modules, framework caches and package stores' },
  { id: 'Python', name: 'Python', emoji: '🐍', color: '#e9c24a', blurb: 'Virtual environments and package caches' },
  { id: 'Swift', name: 'Swift', emoji: '🕊️', color: '#f47c4c', blurb: 'SwiftPM builds and package caches' },
  { id: 'Xcode', name: 'Xcode', emoji: '🔨', color: '#3b8ef0', blurb: 'DerivedData, simulators and device support' },
  { id: 'Docker', name: 'Docker', emoji: '🐳', color: '#2496ed', blurb: 'Build cache and unused images' },
  { id: 'Homebrew', name: 'Homebrew', emoji: '🍺', color: '#f5b23f', blurb: 'Downloads kept after installing' },
  { id: 'Go', name: 'Go', emoji: '🐹', color: '#29b6d8', blurb: 'Go build cache' },
  { id: 'Java & Android', name: 'Java & Android', emoji: '☕️', color: '#9cba5a', blurb: 'Gradle caches' },
  { id: 'AI agents', name: 'AI agents', emoji: '🤖', color: '#dfa38a', blurb: 'Old Claude Code and Cursor versions' },
  { id: 'AI models', name: 'AI models', emoji: '🧠', color: '#ef8fc6', blurb: 'Downloaded models' },
  { id: 'Browsers', name: 'Browsers', emoji: '🌐', color: '#7cbded', blurb: 'Web caches' },
  { id: 'Apps', name: 'App caches', emoji: '🧩', color: '#b39bfa', blurb: 'Caches apps refill on their own' },
  { id: 'macOS', name: 'Trash & system', emoji: '🗑️', color: '#9aa4b2', blurb: 'Trash, old logs and installers' },
  { id: 'Other', name: 'Other', emoji: '📁', color: '#8a8f99', blurb: 'Everything else' },
];

export const program = (id: string): Program => PROGRAMS.find((p) => p.id === id) ?? { ...PROGRAMS[PROGRAMS.length - 1], id, name: id };

export interface Cluster { program: Program; groups: ClusterGroup[]; total: number; count: number }
/** A kind of thing inside a program: "Build folders", "node_modules". */
export interface ClusterGroup extends Omit<QuickGroup, 'items'> { items: ClusterItem[] }
export interface ClusterItem extends QuickItem { /** A tool command to run instead of selecting the item. */ command?: string | null }

export function clusterGroups(groups: ClusterGroup[]): Cluster[] {
  const map = new Map<string, Cluster>();
  for (const g of groups) {
    const p = program(g.program);
    const c = map.get(p.id) ?? { program: p, groups: [], total: 0, count: 0 };
    c.groups.push(g);
    c.total += g.total;
    c.count += g.items.length;
    map.set(p.id, c);
  }
  const out = [...map.values()];
  for (const c of out) c.groups.sort((a, b) => b.total - a.total);
  return out.sort((a, b) => b.total - a.total);
}

// ---------------------------------------------------------------------------
// Scan-based junk → the same clusters
// ---------------------------------------------------------------------------

const BY_NAME: Record<string, [string, string]> = {
  node_modules: ['Node.js', 'node_modules'], '.next': ['Node.js', 'Framework build caches'], '.nuxt': ['Node.js', 'Framework build caches'],
  '.svelte-kit': ['Node.js', 'Framework build caches'], '.turbo': ['Node.js', 'Framework build caches'], '.parcel-cache': ['Node.js', 'Framework build caches'],
  '.angular': ['Node.js', 'Framework build caches'], '.npm': ['Node.js', 'Package caches'], _cacache: ['Node.js', 'Package caches'],
  target: ['Rust', 'Build folders'], '.venv': ['Python', 'Virtual environments'], venv: ['Python', 'Virtual environments'], '.tox': ['Python', 'Virtual environments'],
  '.nox': ['Python', 'Virtual environments'], __pycache__: ['Python', 'Bytecode'], '.pytest_cache': ['Python', 'Test and lint caches'],
  '.mypy_cache': ['Python', 'Test and lint caches'], '.ruff_cache': ['Python', 'Test and lint caches'], '.build': ['Swift', 'Package builds'],
  Pods: ['Xcode', 'CocoaPods'], '.gradle': ['Java & Android', 'Gradle caches'], 'go-build': ['Go', 'Go build cache'],
  'zig-cache': ['Other', 'Zig caches'], '.zig-cache': ['Other', 'Zig caches'], 'zig-out': ['Other', 'Zig caches'], '.dart_tool': ['Other', 'Dart caches'],
};

function junkPlace(it: JunkItem): [string, string] {
  const p = it.path;
  if (BY_NAME[it.name]) return BY_NAME[it.name];
  if (p.includes('/.cargo/')) return ['Rust', 'Crate downloads'];
  if (p.includes('/DerivedData')) return ['Xcode', 'DerivedData'];
  if (p.includes('/CoreSimulator') || p.includes('/Xcode') || p.includes('com.apple.dt.')) return ['Xcode', 'Xcode & simulator caches'];
  if (p.includes('/Caches/Homebrew')) return ['Homebrew', 'Old downloads'];
  if (/\/(pip|pypoetry|uv)(\/|$)/.test(p)) return ['Python', 'Package caches'];
  if (/\/(Yarn|pnpm|bun|ms-playwright|Cypress|electron|node-gyp)(\/|$)/i.test(p)) return ['Node.js', 'Package caches'];
  if (p.includes('org.swift.swiftpm')) return ['Swift', 'Package cache'];
  if (p.includes('CocoaPods')) return ['Xcode', 'CocoaPods'];
  if (it.tag === 'model-cache' || p.includes('huggingface')) return ['AI models', 'Downloaded models'];
  if (it.tag === 'browser-cache') return ['Browsers', 'Browser caches'];
  if (it.tag === 'old-installer') return ['macOS', 'Old installers'];
  if (it.tag === 'diagnostic' || it.tag === 'stale-log') return ['macOS', 'Logs'];
  if (it.tag === 'app-cache') return ['Apps', 'App caches'];
  if (it.tag === 'build-output' || it.tag === 'build-cache' || it.tag === 'project-cache') return ['Other', 'Build output'];
  if (it.tag === 'generated-environment' || it.tag === 'dependency-cache' || it.tag === 'generated-project-output') return ['Other', 'Dependencies'];
  return ['Other', 'Caches'];
}

const AFTER: Record<string, string> = {
  'Build folders': 'Rebuilt on the next cargo build.', node_modules: 'Run your package manager’s install to get them back.',
  'Virtual environments': 'Recreate with uv sync, poetry install or pip install.', DerivedData: 'Rebuilt the next time you build in Xcode.',
  'Old installers': 'Download them again if you ever need to reinstall.', Logs: 'Only useful while troubleshooting.',
};

function age(secs: number | null): string {
  if (!secs) return '';
  const days = (Date.now() / 1000 - secs) / 86400;
  if (days < 1) return 'changed today';
  if (days < 30) return `${Math.floor(days)} d old`;
  if (days < 365) return `${Math.floor(days / 30)} mo old`;
  return `${Math.floor(days / 365)} yr old`;
}

export function junkToGroups(items: JunkItem[], home: string): ClusterGroup[] {
  const map = new Map<string, ClusterGroup>();
  for (const it of items) {
    const [prog, title] = junkPlace(it);
    const key = `${prog}/${title}`;
    const g = map.get(key) ?? { id: key, program: prog, title, blurb: '', after: AFTER[title] ?? 'Recreated by the app or tool that made it.', method: 'quarantine', command: null, total: 0, items: [] };
    const d = describeJunk(it);
    const where = home && it.path.startsWith(home) ? `~${it.path.slice(home.length)}` : it.path;
    const manual = it.action !== 'quarantine';
    g.items.push({
      id: it.path, path: it.path, label: d.title, detail: [where, age(it.mtime)].filter(Boolean).join(' · '), total: it.total, modified: it.mtime,
      preselect: false, blocked: manual ? (it.tool ? null : it.reason) : null, command: manual ? it.tool : null,
    });
    g.total += it.total;
    map.set(key, g);
  }
  for (const g of map.values()) g.items.sort((a, b) => b.total - a.total);
  return [...map.values()];
}
