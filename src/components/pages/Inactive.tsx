import React, { useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { Clock, Plus, Check, Eye, AppWindow, File, Info, ChevronRight } from 'lucide-react';
import { useStore } from '../../store';
import { api } from '../../lib/api';
import { catMeta, formatBytes, formatDate, shortPath } from '../../lib/format';
import { CategoryChip, Empty, SegmentedControl } from '../ui';
import type { UnusedItem } from '../../types';
import { openItemMenu } from '../ContextMenu';

type Tab = 'files' | 'apps';

export const InactivePage: React.FC = () => {
  const { unusedFiles, unusedApps, isStaged, toggleStage, info, navigate } = useStore();
  const [tab, setTab] = useState<Tab>('files');
  const [minDays, setMinDays] = useState(90);

  const items = useMemo(() => {
    const src = tab === 'files' ? unusedFiles : unusedApps;
    return [...src].filter((i) => i.days_inactive >= minDays).sort((a, b) => b.allocated_bytes - a.allocated_bytes);
  }, [tab, unusedFiles, unusedApps, minDays]);

  const total = items.reduce((a, b) => a + b.allocated_bytes, 0);

  if (unusedFiles.length === 0 && unusedApps.length === 0) {
    return <Empty icon={<Clock className="w-6 h-6" />} title="No inactivity data yet" body="Run a scan and Disko will check large files for recent access, modification and Spotlight last-used dates." />;
  }

  return (
    <div className="h-full flex flex-col min-h-0">
      <div className="px-6 pt-4 pb-3 flex items-center gap-3 flex-shrink-0">
        <SegmentedControl<Tab>
          value={tab}
          onChange={setTab}
          options={[
            { value: 'files', label: <><File className="w-3.5 h-3.5" /> Files <span className="mono opacity-70">{unusedFiles.length}</span></> },
            { value: 'apps', label: <><AppWindow className="w-3.5 h-3.5" /> Apps <span className="mono opacity-70">{unusedApps.length}</span></> },
          ]}
        />
        <div className="flex items-center gap-1 text-[12px]" style={{ color: 'var(--muted)' }}>
          <span>Untouched for</span>
          {[90, 180, 365].map((d) => (
            <button key={d} onClick={() => setMinDays(d)} className="chip transition-colors" style={{ padding: '3px 8px', fontSize: 11, background: minDays === d ? 'var(--bg-3)' : 'transparent', color: minDays === d ? 'var(--text)' : 'var(--muted)' }}>
              {d === 365 ? '1 year' : `${d} days`}
            </button>
          ))}
        </div>
        <span className="ml-auto text-[12px]" style={{ color: 'var(--muted)' }}>
          <span className="mono text-white font-semibold">{formatBytes(total)}</span> in {items.length} items
        </span>
      </div>

      <div className="px-6 pb-3 flex-shrink-0">
        <div className="card px-4 py-2.5 flex items-center gap-2.5 text-[12px]" style={{ color: 'var(--muted)' }}>
          <Info className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--warn)' }} />
          {tab === 'files'
            ? 'Inactivity is evidence, not proof. Access times can be stale or disabled. Treat these as things worth a look, not a delete list.'
            : 'Apps live outside your home folder, so Disko will not move them. Use this list to decide what to uninstall yourself. Unknown usage means macOS has no record, not that the app is unused.'}
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-6 pb-6">
        <div className="grid text-[10.5px] uppercase tracking-[0.1em] font-semibold px-3 pb-2" style={{ gridTemplateColumns: '1fr 150px 110px 120px 96px', color: 'var(--dim)' }}>
          <span>Name</span>
          <span>Last evidence</span>
          <span>Inactive</span>
          <span className="text-right">Size</span>
          <span />
        </div>
        {items.map((it, i) => (
          <Row key={it.path} it={it} i={i} home={info?.home ?? ''} staged={isStaged(it.path)} onStage={() => toggleStage({ path: it.path, name: it.name, kind: it.kind === 'app' ? 'directory' : 'file', total: it.allocated_bytes, category: it.category, reason: it.reason, source: 'inactive' })} onOpenFolder={() => navigate(it.path.slice(0, it.path.lastIndexOf('/')))} />
        ))}
        {items.length === 0 && <div className="py-16 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>Nothing older than {minDays} days.</div>}
      </div>
    </div>
  );
};

const Row: React.FC<{ it: UnusedItem; i: number; home: string; staged: boolean; onStage: () => void; onOpenFolder: () => void }> = ({ it, i, home, staged, onStage, onOpenFolder }) => {
  const m = catMeta(it.category);
  const unknown = it.evidence === 'unknown_app_usage';
  const canStage = it.category !== 'protected';
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: Math.min(i * 0.015, 0.4) }}
      className="group grid items-center px-3 py-2.5 rounded-xl row-hover"
      style={{ gridTemplateColumns: '1fr 150px 110px 120px 96px' }}
      onContextMenu={(e) => openItemMenu(e, { path: it.path, name: it.name, kind: it.kind === 'app' ? 'directory' : 'file', total: it.allocated_bytes, category: it.category, reason: it.reason, source: 'inactive' })}
    >
      <div className="flex items-center gap-2.5 min-w-0">
        <span className="w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0" style={{ background: m.soft, color: m.text }}>
          {it.kind === 'app' ? <AppWindow className="w-4 h-4" /> : <File className="w-4 h-4" />}
        </span>
        <div className="min-w-0">
          <div className="flex items-center gap-2 min-w-0">
            <span className={`text-[13px] font-medium truncate ${staged ? 'line-through opacity-60' : ''}`}>{it.name}</span>
            <CategoryChip category={it.category} small />
          </div>
          <div className="mono text-[10.5px] truncate selectable" style={{ color: 'var(--dim)' }} title={it.path}>{shortPath(it.path, home)}</div>
        </div>
      </div>
      <div className="text-[11.5px]" style={{ color: 'var(--muted)' }}>
        {unknown ? <span style={{ color: 'var(--warn)' }}>Usage unknown</span> : formatDate(it.last_used ?? it.last_accessed ?? it.modified)}
        <div className="text-[10px]" style={{ color: 'var(--dim)' }}>{it.last_used ? 'Spotlight last used' : it.last_accessed ? 'last accessed' : 'modified'}</div>
      </div>
      <div className="mono text-[13px] font-semibold" style={{ color: it.days_inactive > 365 ? 'var(--danger)' : it.days_inactive > 180 ? 'var(--warn)' : undefined }}>
        {it.days_inactive}d
      </div>
      <div className="text-right mono text-[13px] font-semibold">{formatBytes(it.allocated_bytes)}</div>
      <div className="flex items-center justify-end gap-1">
        {it.kind !== 'app' && (
          <button className="btn btn-ghost btn-icon opacity-0 group-hover:opacity-100 transition-opacity" title="Open containing folder" onClick={onOpenFolder}>
            <ChevronRight className="w-3.5 h-3.5" />
          </button>
        )}
        <button className="btn btn-ghost btn-icon opacity-0 group-hover:opacity-100 transition-opacity" title="Reveal in Finder" onClick={() => api.reveal(it.path).catch(() => {})}>
          <Eye className="w-3.5 h-3.5" />
        </button>
        <button
          className={`btn btn-icon ${staged ? '' : 'opacity-0 group-hover:opacity-100'} transition-opacity`}
          disabled={!canStage}
          title={canStage ? (staged ? 'Remove from collection' : 'Collect') : 'Protected: uninstall this yourself'}
          style={staged ? { color: 'var(--text)', borderColor: 'var(--line-2)' } : undefined}
          onClick={onStage}
        >
          {staged ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
        </button>
      </div>
    </motion.div>
  );
};
