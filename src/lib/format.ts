// Sizes use decimal units like Finder and About This Mac.
const UNITS = ['bytes', 'KB', 'MB', 'GB', 'TB'];

export function splitBytes(n: number): { value: string; unit: string } {
  if (!n || n < 1) return { value: '0', unit: 'KB' };
  let i = Math.min(UNITS.length - 1, Math.floor(Math.log10(n) / 3));
  if (i === 0) i = 1;
  const v = n / 1000 ** i;
  const value = v >= 100 ? Math.round(v).toString() : v >= 10 ? v.toFixed(1) : v.toFixed(v < 1 ? 2 : 1);
  return { value: value.replace(/\.0+$/, ''), unit: UNITS[i] };
}

export function bytes(n: number): string {
  const { value, unit } = splitBytes(n);
  return `${value} ${unit}`;
}

const now = () => Date.now() / 1000;

/** "today", "3 days", "8 months" — short, for size rows. */
export function age(ts?: number | null): string {
  if (!ts) return '';
  const s = Math.max(0, now() - ts);
  const d = s / 86400;
  if (s < 3600) return 'just now';
  if (d < 1) return `${Math.round(s / 3600)} h ago`;
  if (d < 2) return 'yesterday';
  if (d < 30) return `${Math.round(d)} days ago`;
  if (d < 365) return `${Math.round(d / 30.4)} mo ago`;
  const y = d / 365;
  return `${y < 2 ? y.toFixed(1).replace('.0', '') : Math.round(y)} yr ago`;
}

export function date(ts?: number | null): string {
  if (!ts) return '';
  const dt = new Date(ts * 1000);
  const sameYear = dt.getFullYear() === new Date().getFullYear();
  return dt.toLocaleDateString(undefined, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) });
}

export function daysSince(ts?: number | null): number {
  return ts ? (now() - ts) / 86400 : Infinity;
}

export function count(n: number): string {
  return n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e4 ? `${Math.round(n / 1e3)}K` : n.toLocaleString();
}

export const basename = (p: string) => p.slice(p.lastIndexOf('/') + 1) || p;
export const parentOf = (p: string) => (p === '/' ? null : p.slice(0, p.lastIndexOf('/')) || '/');
export const isWithin = (p: string, dir: string) => p === dir || (dir === '/' ? p.startsWith('/') : p.startsWith(dir + '/'));
