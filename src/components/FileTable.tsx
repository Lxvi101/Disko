import React, { useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { ArrowDown, ArrowUp, Eye, Folder, File, Plus, Check, ChevronRight, Search } from 'lucide-react';
import type { FileItem } from '../types';
import { catMeta, formatBytes } from '../lib/format';
import { CategoryChip } from './ui';
import { openItemMenu } from './ContextMenu';

interface Props {
  items: FileItem[];
  onNavigate: (path: string) => void;
  onSelect: (path: string | null) => void;
  selectedPath: string | null;
  isStaged: (path: string) => boolean;
  onToggleStage: (item: FileItem) => void;
  onReveal: (path: string) => void;
  showFilters?: boolean;
  parentTotal?: number;
}

type SortKey = 'total' | 'name' | 'category';

export const FileTable: React.FC<Props> = ({
  items,
  onNavigate,
  onSelect,
  selectedPath,
  isStaged,
  onToggleStage,
  onReveal,
  showFilters = true,
  parentTotal,
}) => {
  const [q, setQ] = useState('');
  const [cat, setCat] = useState<string>('all');
  const [sort, setSort] = useState<SortKey>('total');
  const [desc, setDesc] = useState(true);

  const total = (parentTotal ?? items.reduce((a, b) => a + b.total, 0)) || 1;

  const rows = useMemo(() => {
    let r = items;
    if (q) {
      const s = q.toLowerCase();
      r = r.filter((i) => i.name.toLowerCase().includes(s));
    }
    if (cat !== 'all') r = r.filter((i) => i.category === cat);
    r = [...r].sort((a, b) => {
      let c = 0;
      if (sort === 'total') c = a.total - b.total;
      else if (sort === 'name') c = a.name.localeCompare(b.name);
      else c = a.category.localeCompare(b.category);
      return desc ? -c : c;
    });
    return r;
  }, [items, q, cat, sort, desc]);

  const toggleSort = (k: SortKey) => {
    if (sort === k) setDesc(!desc);
    else {
      setSort(k);
      setDesc(k === 'total');
    }
  };

  const SortIcon = ({ k }: { k: SortKey }) =>
    sort === k ? desc ? <ArrowDown className="w-3 h-3" /> : <ArrowUp className="w-3 h-3" /> : null;

  return (
    <div className="flex flex-col h-full min-h-0">
      {showFilters && (
        <div className="flex items-center gap-2 pb-3 flex-shrink-0">
          <div className="relative flex-1 max-w-xs">
            <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--dim)' }} />
            <input className="input !pl-8 !py-1.5 !text-[12.5px]" placeholder="Filter this folder…" value={q} onChange={(e) => setQ(e.target.value)} />
          </div>
          <div className="flex items-center gap-1">
            {['all', 'rebuildable', 'review', 'keep', 'protected'].map((c) => {
              const active = cat === c;
              const m = c === 'all' ? null : catMeta(c);
              return (
                <button
                  key={c}
                  onClick={() => setCat(c)}
                  className="chip transition-colors"
                  style={{
                    background: active ? (m ? m.soft : 'rgba(255,255,255,0.1)') : 'transparent',
                    color: active ? (m ? m.text : '#fff') : 'var(--muted)',
                    borderColor: active ? (m ? m.ring : 'var(--line-2)') : 'var(--line)',
                    padding: '4px 10px',
                    fontSize: 11,
                  }}
                >
                  {m && <span className="w-1.5 h-1.5 rounded-full" style={{ background: m.color }} />}
                  {c === 'all' ? 'All' : m!.label}
                </button>
              );
            })}
          </div>
          <span className="ml-auto text-[11.5px] tnum" style={{ color: 'var(--dim)' }}>
            {rows.length} of {items.length}
          </span>
        </div>
      )}

      <div className="grid text-[10.5px] uppercase tracking-[0.1em] font-semibold px-3 pb-2 flex-shrink-0" style={{ gridTemplateColumns: '1fr 120px 110px 88px', color: 'var(--dim)' }}>
        <button className="flex items-center gap-1 text-left hover:text-white transition-colors" onClick={() => toggleSort('name')}>
          Name <SortIcon k="name" />
        </button>
        <button className="flex items-center gap-1 hover:text-white transition-colors" onClick={() => toggleSort('category')}>
          Category <SortIcon k="category" />
        </button>
        <button className="flex items-center gap-1 justify-end hover:text-white transition-colors" onClick={() => toggleSort('total')}>
          Size <SortIcon k="total" />
        </button>
        <span />
      </div>

      <div className="flex-1 overflow-y-auto min-h-0 -mx-1 px-1">
        {rows.map((it, i) => {
          const staged = isStaged(it.path);
          const selected = selectedPath === it.path;
          const pct = (it.total / total) * 100;
          const m = catMeta(it.category);
          return (
            <motion.div
              key={it.path}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: Math.min(i * 0.015, 0.4), duration: 0.2 }}
              className={`group grid items-center px-3 py-2 rounded-xl row-hover cursor-default relative ${selected ? 'bg-white/[0.06]' : ''}`}
              style={{ gridTemplateColumns: '1fr 120px 110px 88px' }}
              onClick={() => onSelect(it.path)}
              onDoubleClick={() => it.kind === 'directory' && onNavigate(it.path)}
              onContextMenu={(e) => openItemMenu(e, it)}
            >
              <div className="flex items-center gap-2.5 min-w-0">
                <span className="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0" style={{ background: m.soft, color: m.text }}>
                  {it.kind === 'directory' ? <Folder className="w-3.5 h-3.5" /> : <File className="w-3.5 h-3.5" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 min-w-0">
                    <span className={`text-[13px] font-medium truncate ${staged ? 'line-through opacity-60' : ''}`}>{it.name}</span>
                    {it.kind === 'directory' && (
                      <button
                        className="opacity-0 group-hover:opacity-100 transition-opacity btn btn-ghost !p-0.5 !rounded-md"
                        onClick={(e) => {
                          e.stopPropagation();
                          onNavigate(it.path);
                        }}
                        title="Open folder"
                      >
                        <ChevronRight className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                  <div className="mt-1 h-[3px] rounded-full overflow-hidden" style={{ background: 'rgba(255,255,255,0.05)', width: '70%' }}>
                    <motion.div className="h-full rounded-full" style={{ background: m.color }} initial={{ width: 0 }} animate={{ width: `${Math.max(pct, 0.5)}%` }} transition={{ type: 'spring', stiffness: 120, damping: 24 }} />
                  </div>
                </div>
              </div>
              <div>
                <CategoryChip category={it.category} small />
              </div>
              <div className="text-right">
                <div className="mono text-[13px] font-semibold">{formatBytes(it.total)}</div>
                <div className="text-[10.5px] tnum" style={{ color: 'var(--dim)' }}>{pct < 0.1 ? '<0.1' : pct.toFixed(1)}%</div>
              </div>
              <div className="flex items-center justify-end gap-1">
                <button
                  className="btn btn-ghost btn-icon opacity-0 group-hover:opacity-100 transition-opacity"
                  title="Reveal in Finder"
                  onClick={(e) => {
                    e.stopPropagation();
                    onReveal(it.path);
                  }}
                >
                  <Eye className="w-3.5 h-3.5" />
                </button>
                <button
                  className={`btn btn-icon ${staged ? '' : 'opacity-0 group-hover:opacity-100'} transition-opacity`}
                  style={staged ? { color: 'var(--text)', borderColor: 'var(--line-2)' } : undefined}
                  title={it.category === 'protected' ? it.reason || 'Protected; cannot be collected' : staged ? 'Remove from collection' : 'Collect'}
                  disabled={it.category === 'protected'}
                  onClick={(e) => {
                    e.stopPropagation();
                    onToggleStage(it);
                  }}
                >
                  {staged ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
                </button>
              </div>
            </motion.div>
          );
        })}
        {rows.length === 0 && (
          <div className="py-12 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>
            Nothing matches.
          </div>
        )}
      </div>
    </div>
  );
};
