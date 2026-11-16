export function formatBytes(bytes: number, decimals = 1, compact = true): string {
  if (!bytes || bytes <= 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
  const v = bytes / Math.pow(k, i);
  return `${v.toFixed(i === 0 ? 0 : compact && v >= 100 ? 0 : decimals)} ${sizes[i]}`;
}

export function splitBytes(bytes: number): { value: string; unit: string } {
  const s = formatBytes(bytes);
  const [value, unit] = s.split(' ');
  return { value, unit };
}

export function formatNumber(n: number): string {
  return new Intl.NumberFormat().format(n);
}

export function formatDate(secondsOrIso: number | string | null | undefined): string {
  if (!secondsOrIso) return '—';
  const d = typeof secondsOrIso === 'number' ? new Date(secondsOrIso * 1000) : new Date(secondsOrIso);
  if (isNaN(d.getTime())) return '—';
  return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export function relativeTime(seconds: number | null | undefined): string {
  if (!seconds) return '—';
  const diff = Date.now() / 1000 - seconds;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  const days = Math.floor(diff / 86400);
  if (days < 30) return `${days} d ago`;
  return new Date(seconds * 1000).toLocaleDateString();
}

export function formatDuration(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return `${m}m ${s}s`;
}

export function shortPath(path: string, home: string): string {
  if (home && path.startsWith(home)) return '~' + path.slice(home.length);
  return path;
}

const TAG_LABELS: Record<string, string> = {
  'app-cache': 'App cache',
  'browser-cache': 'Browser cache',
  'browser-profile': 'Browser profile',
  'build-output': 'Build output',
  'build-cache': 'Build cache',
  'xcode-cache': 'Xcode cache',
  'simulator-cache': 'Simulator cache',
  'simulator-device': 'Simulator devices',
  'dependency-cache': 'Dependency cache',
  'dependency-repository': 'Dependency repository',
  'managed-cleanup': 'Managed by tool',
  'managed-container-store': 'Container store',
  'virtual-machine': 'Virtual machine',
  'virtual-device': 'Virtual device',
  'unused-runtime': 'Runtime version',
  'generated-project-output': 'Generated output',
  'generated-environment': 'Environment',
  'project-cache': 'Project cache',
  'model-cache': 'Model cache',
  'downloaded-model': 'Downloaded model',
  'old-backup': 'Device backup',
  'old-installer': 'Installer',
  'release-artifact': 'Release archive',
  'test-runtime': 'Test devices',
  diagnostic: 'Logs and reports',
  'stale-log': 'Old logs',
  'app-state': 'App state',
  'shared-app-state': 'Shared app state',
  'managed-library': 'Media library',
  'redownloadable-apple-content': 'Apple content',
  'tcc-gated': 'Private data',
  credential: 'Credentials',
  'cloud-managed': 'Cloud synced',
  repository: 'Repository',
  downloads: 'Downloads',
  'user-files': 'User files',
  'user-data': 'User data',
  'orphaned-app': 'Uninstalled app',
  'orphaned-service': 'Launch agent',
  'system-managed': 'System managed',
  'existing-trash': 'In Trash',
  'outside-home': 'Outside home',
};

export function tagLabel(tag: string | undefined | null): string {
  if (!tag) return '';
  return TAG_LABELS[tag] ?? tag.replace(/-/g, ' ').replace(/^./, (c) => c.toUpperCase());
}

export const ACTION_LABELS: Record<string, string> = {
  quarantine: 'Collect',
  manager_command: 'Use the tool',
  owner_app: 'Use the app',
  report_only: 'Explain only',
};

export const COST_LABELS: Record<string, string> = {
  instant: 'Recreated instantly',
  'local-rebuild': 'Rebuilt locally',
  'network-small': 'Small re-download',
  'network-large': 'Large re-download',
  unknown: 'Recreate cost unknown',
  irreplaceable: 'Cannot be recreated',
};

export function parentPath(path: string): string {
  const idx = path.lastIndexOf('/');
  if (idx <= 0) return '/';
  return path.slice(0, idx);
}

export const CATEGORY_META: Record<
  string,
  { label: string; color: string; soft: string; text: string; ring: string; description: string }
> = {
  rebuildable: {
    label: 'Rebuildable',
    color: '#34d399',
    soft: 'rgba(52, 211, 153, 0.14)',
    text: '#6ee7b7',
    ring: 'rgba(52, 211, 153, 0.35)',
    description: 'Caches and build output that apps recreate on demand.',
  },
  review: {
    label: 'Review',
    color: '#fbbf24',
    soft: 'rgba(251, 191, 36, 0.14)',
    text: '#fcd34d',
    ring: 'rgba(251, 191, 36, 0.35)',
    description: 'User data or unknown content. Needs a human decision.',
  },
  keep: {
    label: 'Keep',
    color: '#60a5fa',
    soft: 'rgba(96, 165, 250, 0.14)',
    text: '#93c5fd',
    ring: 'rgba(96, 165, 250, 0.35)',
    description: 'App state, repositories, and managed libraries.',
  },
  protected: {
    label: 'Protected',
    color: '#fb7185',
    soft: 'rgba(251, 113, 133, 0.14)',
    text: '#fda4af',
    ring: 'rgba(251, 113, 133, 0.35)',
    description: 'System, credentials, cloud-managed content. Never touched.',
  },
  other: {
    label: 'Other',
    color: '#64748b',
    soft: 'rgba(100, 116, 139, 0.14)',
    text: '#94a3b8',
    ring: 'rgba(100, 116, 139, 0.35)',
    description: 'Smaller items grouped together.',
  },
};

export function catMeta(cat: string) {
  return CATEGORY_META[cat] ?? CATEGORY_META.review;
}

/** Shift a hex color's lightness by `amt` (-1..1). */
export function shade(hex: string, amt: number): string {
  const n = parseInt(hex.replace('#', ''), 16);
  let r = (n >> 16) & 255;
  let g = (n >> 8) & 255;
  let b = n & 255;
  const t = amt < 0 ? 0 : 255;
  const p = Math.abs(amt);
  r = Math.round((t - r) * p + r);
  g = Math.round((t - g) * p + g);
  b = Math.round((t - b) * p + b);
  return `rgb(${r}, ${g}, ${b})`;
}

export function uid(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36);
}
