import React, { useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Archive, RotateCcw, Flame, Eye, Trash2, CheckCircle2, AlertTriangle, ChevronDown } from 'lucide-react';
import { useStore } from '../../store';
import { api, errorText } from '../../lib/api';
import { formatBytes, formatDate, relativeTime, shortPath } from '../../lib/format';
import { Empty, Modal, Spinner } from '../ui';
import type { QuarantineJournal } from '../../types';

export const QuarantinePage: React.FC = () => {
  const { journals, refreshJournals, refreshCurrent, reloadData, toast, info } = useStore();
  const [busy, setBusy] = useState<string | null>(null);
  const [purgeTarget, setPurgeTarget] = useState<QuarantineJournal | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  const active = useMemo(() => journals.filter((j) => j.items.some((i) => i.present)), [journals]);
  const past = useMemo(() => journals.filter((j) => !j.items.some((i) => i.present)), [journals]);
  const heldBytes = active.reduce((a, j) => a + j.items.filter((i) => i.present).reduce((x, y) => x + y.size, 0), 0);

  const restore = async (j: QuarantineJournal) => {
    setBusy(j.path);
    try {
      const n = await api.restore(j.path);
      toast({ kind: 'success', title: `Restored ${n} item${n === 1 ? '' : 's'}`, detail: 'Everything is back where it was.' });
      await Promise.all([reloadData(), refreshCurrent()]);
    } catch (e) {
      toast({ kind: 'error', title: 'Restore failed', detail: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const purge = async () => {
    if (!purgeTarget) return;
    const j = purgeTarget;
    setBusy(j.path);
    try {
      const freed = await api.purge(j.path);
      toast({ kind: 'success', title: `Freed ${formatBytes(freed)}`, detail: 'Items permanently deleted from quarantine.' });
      setPurgeTarget(null);
      await refreshJournals();
      const s = useStore.getState();
      s.boot();
    } catch (e) {
      toast({ kind: 'error', title: 'Purge failed', detail: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const remove = async (j: QuarantineJournal) => {
    try {
      await api.deleteJournal(j.path);
      await refreshJournals();
    } catch (e) {
      toast({ kind: 'error', title: 'Could not remove record', detail: errorText(e) });
    }
  };

  if (journals.length === 0) {
    return <Empty icon={<Archive className="w-6 h-6" />} title="Quarantine is empty" body="When you clean items, they are moved here first. Restore them any time, or purge to actually free the space." />;
  }

  return (
    <div className="h-full overflow-y-auto px-6 pt-4 pb-8">
      <div className="max-w-[980px] mx-auto">
        <div className="card p-4 flex items-center gap-4 mb-4" >
          <div className="w-11 h-11 rounded-2xl glass flex items-center justify-center" style={{ color: 'var(--muted)' }}>
            <Archive className="w-5 h-5" />
          </div>
          <div className="flex-1">
            <div className="text-[13.5px] font-semibold">
              {active.length ? `${formatBytes(heldBytes)} waiting in quarantine` : 'Nothing waiting in quarantine'}
            </div>
            <div className="text-[12px] mt-0.5" style={{ color: 'var(--muted)' }}>
              Quarantine does not free space by itself. Purge a batch once you are sure you do not need it back.
            </div>
          </div>
        </div>

        {active.length > 0 && <h3 className="label px-1 mb-2">Held</h3>}
        <div className="space-y-2.5">
          <AnimatePresence initial={false}>
            {active.map((j) => (
              <JournalCard key={j.path} j={j} home={info?.home ?? ''} busy={busy === j.path} expanded={expanded === j.path} onToggle={() => setExpanded(expanded === j.path ? null : j.path)} onRestore={() => restore(j)} onPurge={() => setPurgeTarget(j)} />
            ))}
          </AnimatePresence>
        </div>

        {past.length > 0 && (
          <>
            <h3 className="label px-1 mt-6 mb-2">History</h3>
            <div className="space-y-2">
              {past.map((j) => (
                <motion.div key={j.path} layout className="card px-4 py-3 flex items-center gap-3" style={{ opacity: 0.8 }}>
                  <CheckCircle2 className="w-4 h-4 flex-shrink-0" style={{ color: j.purged ? 'var(--ok)' : 'var(--muted)' }} />
                  <div className="flex-1 min-w-0">
                    <div className="text-[12.5px] font-medium">
                      {j.purged ? 'Purged' : 'Restored'} {j.items.length} item{j.items.length === 1 ? '' : 's'} · {formatBytes(j.total_bytes)}
                    </div>
                    <div className="text-[11px]" style={{ color: 'var(--dim)' }}>{formatDate(j.created_at)}</div>
                  </div>
                  <button className="btn btn-ghost btn-sm" onClick={() => remove(j)}>
                    <Trash2 className="w-3 h-3" /> Remove record
                  </button>
                </motion.div>
              ))}
            </div>
          </>
        )}
      </div>

      <Modal
        open={!!purgeTarget}
        onClose={() => setPurgeTarget(null)}
        title="Permanently delete this batch?"
        subtitle="This cannot be undone. Only items inside Disko's quarantine folder are touched."
        icon={<Flame className="w-5 h-5" style={{ color: 'var(--danger)' }} />}
        locked={!!busy}
        footer={
          <>
            <button className="btn" onClick={() => setPurgeTarget(null)} disabled={!!busy}>Cancel</button>
            <button className="btn btn-danger" onClick={purge} disabled={!!busy}>
              {busy ? <Spinner /> : <Flame className="w-3.5 h-3.5" />} Purge {purgeTarget ? formatBytes(purgeTarget.items.filter((i) => i.present).reduce((a, b) => a + b.size, 0)) : ''}
            </button>
          </>
        }
      >
        {purgeTarget && (
          <div className="space-y-1 max-h-64 overflow-y-auto">
            {purgeTarget.items.filter((i) => i.present).map((it) => (
              <div key={it.quarantined} className="flex items-center justify-between gap-3 text-[12px] px-2 py-1.5 rounded-lg" style={{ background: 'var(--bg-2)' }}>
                <span className="mono truncate" style={{ color: 'var(--muted)' }}>{shortPath(it.original, info?.home ?? '')}</span>
                <span className="mono font-semibold flex-shrink-0">{formatBytes(it.size)}</span>
              </div>
            ))}
          </div>
        )}
      </Modal>
    </div>
  );
};

const JournalCard: React.FC<{ j: QuarantineJournal; home: string; busy: boolean; expanded: boolean; onToggle: () => void; onRestore: () => void; onPurge: () => void }> = ({ j, home, busy, expanded, onToggle, onRestore, onPurge }) => {
  const present = j.items.filter((i) => i.present);
  const missing = j.items.length - present.length;
  const bytes = present.reduce((a, b) => a + b.size, 0);
  return (
    <motion.div layout initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, scale: 0.98 }} className="card overflow-hidden">
      <div className="p-4 flex items-center gap-4">
        <div className="w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0" style={{ background: 'var(--bg-3)', color: 'var(--muted)' }}>
          <Archive className="w-4.5 h-4.5" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-baseline gap-2">
            <span className="mono text-[16px] font-semibold">{formatBytes(bytes)}</span>
            <span className="text-[12.5px]" style={{ color: 'var(--muted)' }}>
              {present.length} item{present.length === 1 ? '' : 's'} · {relativeTime(j.created_at)}
            </span>
          </div>
          <div className="text-[11px] mt-0.5 truncate" style={{ color: 'var(--dim)' }}>
            {present.slice(0, 3).map((i) => shortPath(i.original, home)).join(' · ')}
            {present.length > 3 ? ` · +${present.length - 3} more` : ''}
          </div>
          {missing > 0 && (
            <div className="text-[11px] mt-1 flex items-center gap-1" style={{ color: 'var(--warn)' }}>
              <AlertTriangle className="w-3 h-3" /> {missing} item{missing === 1 ? '' : 's'} already restored or missing
            </div>
          )}
        </div>
        <div className="flex items-center gap-1.5 flex-shrink-0">
          <button className="btn btn-ghost btn-icon" onClick={onToggle} title="Show items">
            <motion.span animate={{ rotate: expanded ? 180 : 0 }}><ChevronDown className="w-4 h-4" /></motion.span>
          </button>
          <button className="btn btn-ghost btn-icon" title="Reveal in Finder" onClick={() => api.reveal(j.path.replace(/\/journal\.json$/, '')).catch(() => {})}>
            <Eye className="w-4 h-4" />
          </button>
          <button className="btn" onClick={onRestore} disabled={busy}>
            {busy ? <Spinner /> : <RotateCcw className="w-3.5 h-3.5" />} Restore
          </button>
          <button className="btn btn-danger" onClick={onPurge} disabled={busy}>
            <Flame className="w-3.5 h-3.5" /> Purge
          </button>
        </div>
      </div>
      <AnimatePresence>
        {expanded && (
          <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden" style={{ borderTop: '1px solid var(--border)' }}>
            <div className="p-3 space-y-0.5">
              {j.items.map((it) => (
                <div key={it.quarantined} className="flex items-center justify-between gap-3 text-[12px] px-2 py-1.5 rounded-lg row-hover">
                  <span className={`mono truncate ${it.present ? '' : 'line-through opacity-50'}`} style={{ color: 'var(--muted)' }}>{shortPath(it.original, home)}</span>
                  <span className="mono font-semibold flex-shrink-0">{formatBytes(it.size)}</span>
                </div>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
};
