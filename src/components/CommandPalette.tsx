import React, { useEffect, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Search, Folder, File, CornerDownLeft, Plus, Check } from 'lucide-react';
import { useStore } from '../store';
import { useScan } from '../hooks/useScan';
import { api } from '../lib/api';
import { formatBytes, shortPath } from '../lib/format';
import { CategoryChip, Spinner } from './ui';
import type { FileItem } from '../types';

export const CommandPalette: React.FC = () => {
  const { paletteOpen, setPaletteOpen, navigate, select, info, isStaged, toggleStage, setAdvanced, setTheme, resolvedTheme, setStartScreen } = useStore();
  const { scanning } = useScan();
  const [q, setQ] = useState('');
  const [results, setResults] = useState<FileItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [idx, setIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (paletteOpen) {
      setTimeout(() => inputRef.current?.focus(), 30);
    } else {
      setQ('');
      setResults([]);
      setIdx(0);
    }
  }, [paletteOpen]);

  useEffect(() => {
    if (!paletteOpen) return;
    if (q.trim().length < 2) {
      setResults([]);
      return;
    }
    const t = setTimeout(async () => {
      setLoading(true);
      try {
        setResults(await api.search(q.trim(), 40));
        setIdx(0);
      } catch {
        setResults([]);
      } finally {
        setLoading(false);
      }
    }, 180);
    return () => clearTimeout(t);
  }, [q, paletteOpen]);

  const go = (it: FileItem) => {
    setPaletteOpen(false);
    if (it.kind === 'directory') navigate(it.path);
    else {
      navigate(it.path.slice(0, it.path.lastIndexOf('/'))).then(() => select(it.path));
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setIdx((i) => Math.min(i + 1, results.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter' && results[idx]) {
      go(results[idx]);
    } else if (e.key === 'Escape') {
      setPaletteOpen(false);
    }
  };

  return (
    <AnimatePresence>
      {paletteOpen && (
        <motion.div className="fixed inset-0 z-[90] flex items-start justify-center pt-[12vh] px-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.15 }}>
          <div className="absolute inset-0 bg-black/50 backdrop-blur-sm" onClick={() => setPaletteOpen(false)} />
          <motion.div initial={{ opacity: 0, y: -12, scale: 0.98 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: -8, scale: 0.98 }} transition={{ type: 'spring', stiffness: 420, damping: 34 }} className="relative w-full max-w-[640px] glass-strong rounded-3xl overflow-hidden">
            <div className="flex items-center gap-3 px-4 h-14" style={{ borderBottom: '1px solid var(--border)' }}>
              {loading ? <span className="flex-shrink-0" style={{ color: 'var(--text)' }}><Spinner size={16} /></span> : <Search className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--muted)' }} />}
              <input ref={inputRef} value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={onKey} placeholder="Search every scanned file and folder by name or path…" className="flex-1 bg-transparent outline-none text-[14px] selectable" style={{ color: 'var(--text)' }} />
              <span className="kbd">esc</span>
            </div>
            <div className="max-h-[52vh] overflow-y-auto p-2">
              {results.map((it, i) => {
                const active = i === idx;
                const staged = isStaged(it.path);
                return (
                  <div key={it.path} onMouseEnter={() => setIdx(i)} onClick={() => go(it)} className={`flex items-center gap-3 px-3 py-2 rounded-xl cursor-default ${active ? 'bg-white/[0.08]' : ''}`}>
                    <span className="w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0 glass">
                      {it.kind === 'directory' ? <Folder className="w-4 h-4" style={{ color: 'var(--muted)' }} /> : <File className="w-4 h-4" style={{ color: 'var(--muted)' }} />}
                    </span>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-medium truncate">{it.name}</span>
                        <CategoryChip category={it.category} small />
                      </div>
                      <div className="mono text-[10.5px] truncate" style={{ color: 'var(--dim)' }}>{shortPath(it.path, info?.home ?? '')}</div>
                    </div>
                    <span className="mono text-[12.5px] font-semibold">{formatBytes(it.total)}</span>
                    <button
                      className="btn btn-icon !p-1.5"
                      title={it.category === 'protected' ? it.reason || 'Protected; cannot be collected' : staged ? 'Remove from collection' : 'Collect'}
                      disabled={it.category === 'protected'}
                      style={staged ? { color: 'var(--text)', borderColor: 'var(--line-2)' } : undefined}
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleStage({ ...it, source: 'search' });
                      }}
                    >
                      {staged ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
                    </button>
                    {active && <CornerDownLeft className="w-3.5 h-3.5" style={{ color: 'var(--dim)' }} />}
                  </div>
                );
              })}
              {q.trim().length >= 2 && !loading && results.length === 0 && (
                <div className="py-10 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>No matches in the scan.</div>
              )}
              {q.trim().length < 2 && (
                <div className="py-1">
                  {[
                    { label: scanning ? 'Scan in progress…' : 'New scan…', run: () => setStartScreen(true), disabled: scanning },
                    { label: resolvedTheme === 'light' ? 'Switch to dark appearance' : 'Switch to light appearance', run: () => setTheme(resolvedTheme === 'light' ? 'dark' : 'light') },
                    { label: 'Follow system appearance', run: () => setTheme('system') },
                    { label: 'Toggle advanced mode', run: () => setAdvanced(!useStore.getState().advanced) },
                  ].map((a) => (
                    <button key={a.label} disabled={a.disabled} className="w-full text-left px-3 py-2 rounded-lg row-hover text-[13px] disabled:opacity-40" onClick={() => { setPaletteOpen(false); a.run(); }}>
                      {a.label}
                    </button>
                  ))}
                  <div className="px-3 pt-2 pb-1 text-[11px]" style={{ color: 'var(--dim)' }}>Type to search every scanned file and folder.</div>
                </div>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
