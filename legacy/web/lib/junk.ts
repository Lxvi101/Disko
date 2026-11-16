import type { JunkItem } from '../types';

export type JunkGroup = 'deps' | 'build' | 'cache' | 'logs' | 'installers';

export const JUNK_GROUPS: { id: JunkGroup; label: string; color: string; tags: string[]; blurb: string }[] = [
  { id: 'deps', label: 'Dependencies', color: '#7c8cf8', tags: ['generated-project-output', 'generated-environment', 'dependency-cache'], blurb: 'node_modules, Python venvs and downloaded packages. Reinstalled with one command.' },
  { id: 'build', label: 'Build files', color: '#e4a853', tags: ['build-output', 'build-cache', 'project-cache', 'xcode-cache'], blurb: 'Compiled output and build caches. Rebuilt the next time you build.' },
  { id: 'cache', label: 'Caches', color: '#4fb6a8', tags: ['app-cache', 'browser-cache', 'simulator-cache', 'managed-cleanup', 'model-cache'], blurb: 'App, browser and package-manager caches. Refilled as you use them.' },
  { id: 'logs', label: 'Logs', color: '#c98bd9', tags: ['diagnostic', 'stale-log'], blurb: 'App logs and crash reports. Only useful while troubleshooting.' },
  { id: 'installers', label: 'Installers', color: '#e0787a', tags: ['old-installer'], blurb: 'Disk images and archives in Downloads. Once installed, they can go.' },
];

export const junkGroup = (it: Pick<JunkItem, 'tag'>) => JUNK_GROUPS.find((g) => g.tags.includes(it.tag)) ?? JUNK_GROUPS[2];

const KIND: Record<string, string> = {
  node_modules: 'node_modules', '.venv': 'Python venv', venv: 'Python venv', '.tox': 'tox envs', '.nox': 'nox envs', Pods: 'CocoaPods',
  target: 'Rust build', '.build': 'Swift build', '.next': 'Next.js build', '.nuxt': 'Nuxt build', '.svelte-kit': 'SvelteKit build', '.turbo': 'Turbo cache',
  '.parcel-cache': 'Parcel cache', '.angular': 'Angular cache', '.gradle': 'Gradle cache', '.dart_tool': 'Dart cache', __pycache__: 'Python bytecode',
  '.pytest_cache': 'pytest cache', '.mypy_cache': 'mypy cache', '.ruff_cache': 'ruff cache', 'zig-cache': 'Zig cache', '.zig-cache': 'Zig cache', 'zig-out': 'Zig build',
};
const PROJECT_TAGS = ['generated-project-output', 'generated-environment', 'build-output', 'build-cache', 'project-cache'];

/** A human title and subtitle: "my-app" + "node_modules" for project output, the folder itself otherwise. */
export function describeJunk(it: JunkItem): { title: string; kind: string } {
  const parts = it.path.split('/').filter(Boolean);
  const kind = KIND[it.name];
  if (kind && PROJECT_TAGS.includes(it.tag) && parts.length >= 2) return { title: parts[parts.length - 2], kind };
  if (it.path.includes('/DerivedData/') && it.name !== 'ModuleCache.noindex') return { title: it.name.replace(/-[a-z]{20,}$/, ''), kind: 'Xcode build' };
  const parent = parts[parts.length - 2] ?? '';
  const label = it.tag === 'old-installer' ? 'Installer' : it.tag === 'diagnostic' || it.tag === 'stale-log' ? 'Logs' : it.tag === 'managed-cleanup' ? 'Package cache' : it.tag === 'model-cache' ? 'Model cache' : it.tag === 'browser-cache' ? 'Browser cache' : 'Cache';
  // Bundle-style names read better without the reverse-DNS prefix.
  const title = /^(com|net|org|io|app|dev)\.[\w-]+\./.test(it.name) ? it.name.split('.').slice(2).join('.') || it.name : it.name.startsWith('.') || it.name === 'cache' || it.name === 'Cache' ? `${parent} ${it.name}`.trim() : it.name;
  return { title, kind: label };
}
