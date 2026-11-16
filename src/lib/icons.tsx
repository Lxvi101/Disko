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
  node: [Hexagon, '#7cc35a'], rust: [Cog, '#e8875b'], xcode: [Hammer, '#3f8cff'], python: [Code2, '#f2c94c'],
  docker: [Container, '#2f96ff'], brew: [Beer, '#f5a524'], claude: [Sparkle, '#e07a52'], codex: [Terminal, '#22b88a'],
  t3: [GitBranch, '#a58bff'], cursor: [Terminal, '#c7c9d1'], trash: [Trash2, '#aab2c5'], log: [ScrollText, '#c4b08a'],
  box: [Box, '#f2b84b'], app: [AppWindow, '#8a96b3'], cache: [Layers, '#2fd3e6'], iphone: [Smartphone, '#6f8cff'],
  apple: [Apple, '#b8bfcc'], disc: [Disc3, '#34d399'], sparkles: [Sparkles, '#b6e35a'], watch: [Watch, '#8ea2ff'],
  film: [Film, '#e47bff'], archive: [Archive, '#e8b04a'], bun: [Croissant, '#f3d9a4'], folder: [Folder, '#6ea8ff'],
  disk: [HardDrive, '#9aa5bd'],
};

export function glyphOf(key: string): [LucideIcon, string] {
  return GLYPHS[key] ?? GLYPHS.box;
}

/** A soft squircle tile with a glyph, used for findings, agent tools and fallbacks. */
export const Glyph: React.FC<{ name: string; size?: number; color?: string; icon?: LucideIcon; className?: string }> = ({ name, size = 36, color, icon, className }) => {
  const [I, c] = icon ? [icon, color ?? '#8a96b3'] : glyphOf(name);
  const col = color ?? c;
  return (
    <span
      className={`inline-flex items-center justify-center shrink-0 ${className ?? ''}`}
      style={{
        width: size, height: size, borderRadius: size * 0.3,
        background: `linear-gradient(160deg, ${col}38, ${col}14)`,
        boxShadow: `inset 0 0 0 1px ${col}30, inset 0 1px 0 rgba(255,255,255,.08)`,
        color: col,
      }}
    >
      <I size={size * 0.52} strokeWidth={1.9} />
    </span>
  );
};

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
  [/\.(mov|mp4|m4v|avi|mkv|fcpbundle)$/i, Film, '#e47bff'],
  [/\.(png|jpe?g|heic|gif|webp|tiff?|photoslibrary)$/i, Image, '#4fd1c5'],
  [/\.(m4a|mp3|wav|aiff?|flac)$/i, Music, '#ff6fa7'],
  [/\.(zip|xip|tar|gz|tgz|7z|rar|dmg|pkg|ipsw|iso|img|raw)$/i, Archive, '#e8b04a'],
  [/\.(pdf|txt|md|docx?|pages|key|rtf)$/i, FileText, '#9fb3d9'],
  [/\.(js|ts|tsx|rs|py|swift|json|rlib|jsonl)$/i, FileCode, '#8fb4ff'],
];

/** Icon for a tree node: app bundles get their own icon, clutter its category, others by type. */
export const NodeIcon: React.FC<{ node: Pick<TreeNode, 'name' | 'path' | 'kind' | 'clutter' | 'error'>; size?: number; onColor?: boolean }> = ({ node, size = 28, onColor }) => {
  if (node.name.endsWith('.app')) return <Icon iconKey={`app:${node.path}`} size={size} />;
  if (node.kind === 'disk') return <Glyph name="disk" size={size} />;
  if (node.kind === 'hidden') return <Glyph name="x" icon={EyeOff} color="#9b8cc4" size={size} />;
  if (node.error) return <Glyph name="x" icon={Lock} color="#9aa5bd" size={size} />;
  const cat = catOf(node.clutter);
  if (cat) return <Glyph name={cat.id} icon={cat.icon} color={onColor ? '#ffffff' : cat.color} size={size} />;
  if (node.kind === 'other') return <Glyph name="x" icon={Package} color="#8b93a7" size={size} />;
  if (node.kind === 'dir') return <Glyph name="folder" size={size} />;
  const hit = EXT.find(([re]) => re.test(node.name));
  return <Glyph name="x" icon={hit?.[1] ?? File} color={hit?.[2] ?? '#9aa5bd'} size={size} />;
};
