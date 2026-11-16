import {
  Bot, Box, Container, Disc3, Ghost, Hammer, History, Hourglass, Layers, ScrollText, Smartphone, Sparkles, Trash2,
  type LucideIcon,
} from 'lucide-react';
import type { Category } from './types';

export interface CategoryMeta {
  id: Category;
  label: string;
  icon: LucideIcon;
  /** Vivid hue used for fills on dark and light. */
  color: string;
  /** Second stop for gradients. */
  color2: string;
  /** A routine's default interval for this kind of clutter, in days. */
  everyDays: number;
  /** One sentence, shown only on hover. */
  blurb: string;
}

// The one place category colours and icons are defined. Order = the order the UI shows them in.
export const CATEGORIES: CategoryMeta[] = [
  { id: 'build', label: 'Build files', icon: Hammer, color: '#ff8a3d', color2: '#ff5e3a', everyDays: 2, blurb: 'node_modules, Rust targets, DerivedData. Your tools rebuild them.' },
  { id: 'cache', label: 'Caches', icon: Layers, color: '#2fd3e6', color2: '#1ea0ff', everyDays: 7, blurb: 'Apps refill these by themselves.' },
  { id: 'packages', label: 'Tool downloads', icon: Box, color: '#ffc53d', color2: '#ff9f1c', everyDays: 14, blurb: 'Package manager caches: npm, Homebrew, cargo, pip…' },
  { id: 'developer', label: 'Xcode & devices', icon: Smartphone, color: '#5b8cff', color2: '#4361ee', everyDays: 30, blurb: 'Simulators, device symbols and archives.' },
  { id: 'leftovers', label: 'Leftovers', icon: Ghost, color: '#ff5fa2', color2: '#e83f8c', everyDays: 30, blurb: 'Pieces of apps you already removed.' },
  { id: 'old-versions', label: 'Old versions', icon: History, color: '#a98bff', color2: '#7b61ff', everyDays: 30, blurb: 'Older copies of tools you have a newer version of.' },
  { id: 'installers', label: 'Installers', icon: Disc3, color: '#34d399', color2: '#10b981', everyDays: 30, blurb: 'Disk images and installers that already did their job.' },
  { id: 'logs', label: 'Logs', icon: ScrollText, color: '#c4b08a', color2: '#a68a5b', everyDays: 7, blurb: 'Logs and crash reports.' },
  { id: 'agent-data', label: 'Agent data', icon: Bot, color: '#ff7a6b', color2: '#f2545b', everyDays: 14, blurb: 'Coding agent histories and worktrees.' },
  { id: 'containers', label: 'Containers & VMs', icon: Container, color: '#3fa7ff', color2: '#2563eb', everyDays: 30, blurb: 'Docker and virtual machine disks.' },
  { id: 'extras', label: 'Extras', icon: Sparkles, color: '#b6e35a', color2: '#7cc43a', everyDays: 30, blurb: 'AI models, runtimes and other optional downloads.' },
  { id: 'forgotten', label: 'Forgotten', icon: Hourglass, color: '#e47bff', color2: '#b45cff', everyDays: 90, blurb: 'Big files you haven’t opened in a long time.' },
  { id: 'trash', label: 'Trash', icon: Trash2, color: '#aab2c5', color2: '#7d8599', everyDays: 30, blurb: 'Already thrown away. Emptying it is permanent.' },
];

export const CAT: Record<Category, CategoryMeta> = Object.fromEntries(CATEGORIES.map((c) => [c.id, c])) as Record<Category, CategoryMeta>;
export const catOf = (c?: string | null): CategoryMeta | null => (c && c in CAT ? CAT[c as Category] : null);
