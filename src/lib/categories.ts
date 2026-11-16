import {
  Bot, Box, Container, Disc3, Ghost, Hammer, History, Hourglass, Layers, ScrollText, Smartphone, Sparkles, Trash2,
  type LucideIcon,
} from 'lucide-react';
import type { Category } from './types';

export interface CategoryMeta {
  id: Category;
  label: string;
  icon: LucideIcon;
  /** One muted, dusty tone per category, used for fills, chips and bars on dark and light. */
  color: string;
  /** A routine's default interval for this kind of clutter, in days. */
  everyDays: number;
  /** One sentence, shown only on hover. */
  blurb: string;
}

// The one place category colours and icons are defined. Order = the order the UI shows them in.
// One harmonised palette: mid-lightness, low chroma, dusty tones around a cool axis, distinct enough for the maps.
export const CATEGORIES: CategoryMeta[] = [
  { id: 'build', label: 'Build files', icon: Hammer, color: '#c8875c', everyDays: 2, blurb: 'node_modules, Rust targets, DerivedData. Your tools rebuild them.' },
  { id: 'cache', label: 'Caches', icon: Layers, color: '#5c9fa6', everyDays: 7, blurb: 'Apps refill these by themselves.' },
  { id: 'packages', label: 'Tool downloads', icon: Box, color: '#c2a565', everyDays: 14, blurb: 'Package manager caches: npm, Homebrew, cargo, pip…' },
  { id: 'developer', label: 'Xcode & devices', icon: Smartphone, color: '#6e8dcf', everyDays: 30, blurb: 'Simulators, device symbols and archives.' },
  { id: 'leftovers', label: 'Leftovers', icon: Ghost, color: '#c07990', everyDays: 30, blurb: 'Pieces of apps you already removed.' },
  { id: 'old-versions', label: 'Old versions', icon: History, color: '#9187c8', everyDays: 30, blurb: 'Older copies of tools you have a newer version of.' },
  { id: 'installers', label: 'Installers', icon: Disc3, color: '#6fa588', everyDays: 30, blurb: 'Disk images and installers that already did their job.' },
  { id: 'logs', label: 'Logs', icon: ScrollText, color: '#a59781', everyDays: 7, blurb: 'Logs and crash reports.' },
  { id: 'agent-data', label: 'Agent data', icon: Bot, color: '#c4786a', everyDays: 14, blurb: 'Coding agent histories and worktrees.' },
  { id: 'containers', label: 'Containers & VMs', icon: Container, color: '#5a84b8', everyDays: 30, blurb: 'Docker and virtual machine disks.' },
  { id: 'extras', label: 'Extras', icon: Sparkles, color: '#9aa66a', everyDays: 30, blurb: 'AI models, runtimes and other optional downloads.' },
  { id: 'forgotten', label: 'Forgotten', icon: Hourglass, color: '#a57fb2', everyDays: 90, blurb: 'Big files you haven’t opened in a long time.' },
  { id: 'trash', label: 'Trash', icon: Trash2, color: '#8a909c', everyDays: 30, blurb: 'Already thrown away. Emptying it is permanent.' },
];

export const CAT: Record<Category, CategoryMeta> = Object.fromEntries(CATEGORIES.map((c) => [c.id, c])) as Record<Category, CategoryMeta>;
export const catOf = (c?: string | null): CategoryMeta | null => (c && c in CAT ? CAT[c as Category] : null);

/** A category tone mixed toward transparent: `tint(color, 18)` = 18 % of the colour. */
export const tint = (color: string, pct: number) => `color-mix(in srgb, ${color} ${pct}%, transparent)`;
