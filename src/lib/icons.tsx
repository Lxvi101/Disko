import React, { useEffect, useState } from 'react';
import {
  AppWindow, Apple, Archive, Beer, Box, Cog, Container, Croissant, Disc3, EyeOff, File, FileCode, FileText, Film, Folder,
  GitBranch, HardDrive, Hexagon, Image, Layers, Music, Package, ScrollText, Smartphone, Sparkle, Sparkles, Terminal,
  Trash2, Watch, Hammer, Code2, Lock, type LucideIcon,
} from 'lucide-react';
import { api } from './api';
import { catOf } from './categories';
import type { TreeNode } from './types';

/** Known glyph keys from Finding.icon / AgentTool.icon → icon + brand-ish colour. */
const GLYPHS: Record<string, [LucideIcon, string]> = {
  node: [Hexagon, '#86a96a'], rust: [Cog, '#c4886a'], xcode: [Hammer, '#6e8dcf'], python: [Code2, '#c2a565'],
  docker: [Container, '#5a84b8'], brew: [Beer, '#c29a5e'], claude: [Sparkle, '#c4826a'], codex: [Terminal, '#6fa588'],
  t3: [GitBranch, '#9187c8'], cursor: [Terminal, '#a3a8b4'], trash: [Trash2, '#8a909c'], log: [ScrollText, '#a59781'],
  box: [Box, '#c2a565'], app: [AppWindow, '#8b93a6'], cache: [Layers, '#5c9fa6'], iphone: [Smartphone, '#6e8dcf'],
  apple: [Apple, '#a3a8b4'], disc: [Disc3, '#6fa588'], sparkles: [Sparkles, '#9aa66a'], watch: [Watch, '#8394c8'],
  film: [Film, '#a57fb2'], archive: [Archive, '#b99a62'], bun: [Croissant, '#c9b48e'], folder: [Folder, '#7f9bd2'],
  disk: [HardDrive, '#9298a6'],
};

export function glyphOf(key: string): [LucideIcon, string] {
  return GLYPHS[key] ?? GLYPHS.box;
}

/** Pulls a tone toward the text colour so glyphs stay legible on dark and light. */
const ink = (col: string, pct: number) => `color-mix(in oklab, ${col}, var(--glyph-mix) ${pct}%)`;
const tint = (col: string, pct: number) => `color-mix(in srgb, ${col} ${pct}%, transparent)`;

/** A quiet squircle chip with a glyph, used for findings, agent tools and fallbacks. */
export const Glyph: React.FC<{ name: string; size?: number; color?: string; icon?: LucideIcon; className?: string }> = ({ name, size = 36, color, icon, className }) => {
  const [I, c] = icon ? [icon, color ?? '#8b93a6'] : glyphOf(name);
  const col = color ?? c;
  return (
    <span
      className={`inline-flex items-center justify-center shrink-0 ${className ?? ''}`}
      style={{
        width: size, height: size, borderRadius: size * 0.3,
        background: `linear-gradient(180deg, ${tint(col, 20)}, ${tint(col, 10)})`,
        boxShadow: `inset 0 0 0 1px ${tint(col, 20)}, inset 0 1px 0 var(--hi)`,
        color: ink(col, 22),
      }}
    >
      <I size={size * 0.5} strokeWidth={1.7} />
    </span>
  );
};

/**
 * The big section/category icon: frosted translucent glass with a soft inner light and a soft tinted
 * shadow beneath, like a glass folder on a calm field. Never glossy.
 */
export const GlassIcon: React.FC<{ icon: LucideIcon; color: string; size?: number; className?: string }> = ({ icon: I, color, size = 52, className }) => (
  <span
    className={`relative inline-flex items-center justify-center shrink-0 ${className ?? ''}`}
    style={{
      width: size, height: size, borderRadius: size * 0.3,
      background: `linear-gradient(180deg, ${tint(color, 30)}, ${tint(color, 12)} 70%)`,
      border: `1px solid ${tint(color, 26)}`,
      boxShadow: `inset 0 1px 0 var(--hi), inset 0 -${size * 0.18}px ${size * 0.3}px -${size * 0.18}px ${tint(color, 45)}, 0 ${size * 0.22}px ${size * 0.4}px -${size * 0.26}px ${tint(color, 70)}`,
      backdropFilter: 'blur(12px)',
      WebkitBackdropFilter: 'blur(12px)',
      color: ink(color, 38),
    }}
  >
    <I size={size * 0.48} strokeWidth={1.6} />
  </span>
);

const iconCache = new Map<string, Promise<string | null>>();
function loadIcon(key: string, size: number) {
  const k = `${key}@${size}`;
  if (!iconCache.has(k)) iconCache.set(k, api.icon(key, size).catch(() => null));
  return iconCache.get(k)!;
}

/** An app's own icon ("app:<path>"), else a glyph. */
export const Icon: React.FC<{ iconKey: string; size?: number; className?: string }> = ({ iconKey, size = 36, className }) => {
  const isApp = iconKey.startsWith('app:');
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!isApp) return;
    let live = true;
    loadIcon(iconKey, Math.round(size * 2)).then((s) => live && setSrc(s));
    return () => { live = false; };
  }, [iconKey, size, isApp]);
  if (!isApp) return <Glyph name={iconKey} size={size} className={className} />;
  if (!src) return <Glyph name="app" size={size} className={className} />;
  return <img src={src} width={size} height={size} draggable={false} className={`shrink-0 ${className ?? ''}`} style={{ width: size, height: size }} alt="" />;
};

const EXT: [RegExp, LucideIcon, string][] = [
  [/\.(mov|mp4|m4v|avi|mkv|fcpbundle)$/i, Film, '#a57fb2'],
  [/\.(png|jpe?g|heic|gif|webp|tiff?|photoslibrary)$/i, Image, '#5c9fa6'],
  [/\.(m4a|mp3|wav|aiff?|flac)$/i, Music, '#c07990'],
  [/\.(zip|xip|tar|gz|tgz|7z|rar|dmg|pkg|ipsw|iso|img|raw)$/i, Archive, '#b99a62'],
  [/\.(pdf|txt|md|docx?|pages|key|rtf)$/i, FileText, '#8f9fc0'],
  [/\.(js|ts|tsx|rs|py|swift|json|rlib|jsonl)$/i, FileCode, '#7f9bd2'],
];

/** Icon for a tree node: app bundles get their own icon, clutter its category, others by type. */
export const NodeIcon: React.FC<{ node: Pick<TreeNode, 'name' | 'path' | 'kind' | 'clutter' | 'error'>; size?: number; onColor?: boolean }> = ({ node, size = 28, onColor }) => {
  if (node.name.endsWith('.app')) return <Icon iconKey={`app:${node.path}`} size={size} />;
  if (node.kind === 'disk') return <Glyph name="disk" size={size} />;
  if (node.kind === 'hidden') return <Glyph name="x" icon={EyeOff} color="#8e88aa" size={size} />;
  if (node.error) return <Glyph name="x" icon={Lock} color="#9298a6" size={size} />;
  const cat = catOf(node.clutter);
  if (cat) return <Glyph name={cat.id} icon={cat.icon} color={onColor ? '#ffffff' : cat.color} size={size} />;
  if (node.kind === 'other') return <Glyph name="x" icon={Package} color="#8b93a6" size={size} />;
  if (node.kind === 'dir') return <Glyph name="folder" size={size} />;
  const hit = EXT.find(([re]) => re.test(node.name));
  return <Glyph name="x" icon={hit?.[1] ?? File} color={hit?.[2] ?? '#9298a6'} size={size} />;
};
