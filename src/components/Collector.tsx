import React, { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Eye, Sparkles, Archive, Trash2, Check } from 'lucide-react';
import { useStore } from '../store';
import { api, errorText, onCleanupProgress } from '../lib/api';
import { formatBytes, shortPath, catMeta } from '../lib/format';
import { Modal, Spinner } from './ui';
import type { CleanupProgress, StagedItem } from '../types';
import { openItemMenu } from './ContextMenu';

export const Collector: React.FC = () => {
  const { staged, unstage, clearStaged, setCleanModalOpen, info, trayOpen, setTrayOpen, toggleStage, setAssistantOpen, advanced, toast } = useStore();
  const [over, setOver] = useState(false);
  const total = staged.reduce((a, b) => a + b.total, 0);
  const count = staged.length;

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setOver(false);
    const raw = e.dataTransfer.getData('application/x-disko-item');
    if (!raw) return;
    try {
      const it = JSON.parse(raw) as Partial<StagedItem> & { path: string };
      if (it.category === 'protected') {
        toast({ kind: 'info', title: `${it.name ?? it.path} cannot be collected`, detail: it.reason || 'Protected: system, credential or cloud-managed path.' });
        return;
      }
      toggleStage({ path: it.path, name: it.name ?? it.path.split('/').pop() ?? it.path, kind: it.kind ?? 'directory', total: it.total ?? 0, category: it.category ?? 'review', reason: it.reason ?? '', source: 'explore' });
    } catch {}
  };

  const review = () => {
    setAssistantOpen(true);
    window.dispatchEvent(new CustomEvent('disko:ask', { detail: 'Review the items I collected for cleanup. For each one say whether it is clearly disposable, likely abandoned, or needs confirmation, flag anything risky, and give the recreate cost. If a manager command would be the safer removal for any of them, say so.' }));
  };

  return (
    <div className="collector relative flex items-center gap-4 h-[72px] px-6 flex-shrink-0" style={{ borderTop: advanced ? '1px solid var(--line)' : undefined }}>
      {/* target */}
      <motion.button
        onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; setOver(true); }}
        onDragLeave={() => setOver(false)}
        onDrop={onDrop}
        onClick={() => count > 0 && setTrayOpen(!trayOpen)}
        animate={{ scale: over ? 1.08 : 1 }}
        className="relative w-11 h-11 rounded-full flex items-center justify-center flex-shrink-0"
        style={{ border: `1.5px solid ${over ? 'var(--text)' : 'var(--line-2)'}` }}
        id="disko-collector-target"
        title={count ? 'Show collected items' : 'Drop files here'}
      >
        <span className="rounded-full" style={{ width: 28, height: 28, border: `1.5px solid ${over ? 'var(--text)' : 'var(--line-2)'}` }} />
        <AnimatePresence>
          {count > 0 && (
            <motion.span key={count} initial={{ scale: 0.6, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={{ scale: 0.6, opacity: 0 }} className="absolute inset-0 flex items-center justify-center tnum text-[12px] font-semibold">
              {count}
            </motion.span>
          )}
        </AnimatePresence>
      </motion.button>

      <div className="flex-1 min-w-0" onDragOver={(e) => { e.preventDefault(); setOver(true); }} onDragLeave={() => setOver(false)} onDrop={onDrop}>
        <AnimatePresence mode="wait" initial={false}>
          {count === 0 ? (
            <motion.span key="empty" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="text-[13px]" style={{ color: 'var(--dim)' }}>
              Drag and drop files here to collect them
            </motion.span>
          ) : (
            <motion.div key="full" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }} className="flex items-center gap-4">
              <span className="text-[13px]">
                {count} item{count === 1 ? '' : 's'} <span style={{ color: 'var(--muted)' }}>·</span> <span className="tnum">{formatBytes(total)}</span>
              </span>
              <button className="btn btn-ghost btn-sm" onClick={() => setTrayOpen(!trayOpen)}>{trayOpen ? 'Hide' : 'Show'}</button>
              <button className="btn btn-ghost btn-sm" onClick={clearStaged}>Clear</button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {count > 0 && (
        <div className="flex items-center gap-2">
          <button className="btn" onClick={review}>
            <Sparkles className="w-3.5 h-3.5" /> Ask Disko
          </button>
          <button className="btn btn-primary" onClick={() => { useStore.setState({ cleanupSelection: null }); setCleanModalOpen(true); }}>
            Clean up {formatBytes(total)}
          </button>
        </div>
      )}

      {/* popover list */}
      <AnimatePresence>
        {trayOpen && count > 0 && (
          <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 8 }} transition={{ type: 'spring', stiffness: 400, damping: 32 }} className="absolute left-6 bottom-[80px] w-[460px] panel-raised p-2 z-40">
            <div className="max-h-[40vh] overflow-y-auto">
              {staged.map((s) => (
                <div key={s.path} className="group flex items-center gap-2.5 px-2 h-9 rounded-lg row-hover" onContextMenu={(e) => openItemMenu(e, s)}>
                  <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ background: catMeta(s.category).color, opacity: advanced ? 1 : 0.5 }} />
                  <div className="flex-1 min-w-0">
                    <div className="text-[12.5px] truncate">{s.name}</div>
                    {s.action && s.action !== 'quarantine' && s.tool ? (
                      <div className="mono text-[10px] truncate" style={{ color: 'var(--warn)' }} title={`Safer: ${s.tool}`}>safer: {s.tool}</div>
                    ) : advanced ? (
                      <div className="mono text-[10px] truncate" style={{ color: 'var(--dim)' }}>{shortPath(s.path, info?.home ?? '')}</div>
                    ) : null}
                  </div>
                  <span className="tnum text-[12px]" style={{ color: 'var(--muted)' }}>{s.total ? formatBytes(s.total) : '—'}</span>
                  <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100" onClick={() => api.reveal(s.path).catch(() => {})} title="Reveal in Finder"><Eye className="w-3.5 h-3.5" /></button>
                  <button className="btn btn-ghost btn-icon !p-1" onClick={() => unstage(s.path)} title="Remove"><X className="w-3.5 h-3.5" /></button>
                </div>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};

const MODES = {
  quarantine: { label: 'Quarantine', icon: Archive, verb: 'Quarantine', doing: 'Moving to quarantine', note: 'Recoverable from Quarantine. Space is freed when you purge.' },
  delete: { label: 'Delete permanently', icon: Trash2, verb: 'Delete', doing: 'Deleting', note: 'Frees space now. Bypasses Trash and cannot be undone.' },
} as const;

export const CleanModal: React.FC = () => {
  const { staged: collection, cleanupSelection, cleanModalOpen, setCleanModalOpen, refreshCurrent, reloadData, info, setPage, advanced, cleanupMode: mode, cleanupBusy: busy } = useStore();
  const staged = cleanupSelection ?? collection;
  const hasSimulator = staged.some((s) => s.path.includes('/CoreSimulator/Devices/'));
  const [result, setResult] = useState<{ paths: string[]; bytes: number; failures: { path: string; error: string }[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<CleanupProgress | null>(null);
  const [items, setItems] = useState<StagedItem[]>([]);
  const total = staged.reduce((a, b) => a + b.total, 0);
  useEffect(() => { if (cleanModalOpen) { setResult(null); setError(null); setProgress(null); } }, [cleanModalOpen]);
  const close = () => { if (!busy) setCleanModalOpen(false); };
  const m = MODES[mode];
  // Keep the batch on screen while it runs: removing items from the collection must not reshuffle the list.
  const list = busy || result ? items : staged;
  const listTotal = list.reduce((a, b) => a + b.total, 0);
  const finishing = busy && !!progress && progress.finished >= progress.count;
  const fraction = !progress ? 0 : progress.total_bytes > 0 ? progress.done_bytes / progress.total_bytes : progress.finished / Math.max(progress.count, 1);
  const run = async () => {
    if (useStore.getState().cleanupBusy) return;
    useStore.setState({ cleanupBusy: true });
    setError(null);
    setItems(staged);
    setProgress({ index: 0, count: staged.length, finished: 0, path: staged[0]?.path ?? '', done_bytes: 0, total_bytes: total });
    const unlisten = await onCleanupProgress(setProgress).catch(() => null);
    try {
      const items = staged.map((s) => ({ path: s.path, size: s.total }));
      let outcome;
      if (mode === 'delete') {
        const res = await api.deleteItems(items);
        outcome = { paths: res.deleted_items, bytes: res.total_bytes, failures: res.failures };
      } else {
        const res = await api.quarantine(items);
        outcome = { paths: res.moved_items, bytes: res.total_bytes, failures: res.skipped.map((path) => ({ path, error: 'Item no longer exists or its destination is occupied.' })) };
      }
      setResult(outcome);
      useStore.setState((s) => ({ staged: s.staged.filter((i) => !outcome.paths.includes(i.path)) }));
    } catch (e) { setError(errorText(e)); }
    finally {
      unlisten?.();
      const versionBeforeRefresh = useStore.getState().dataVersion;
      await Promise.allSettled([reloadData(), refreshCurrent(), api.volume().then((volume) => useStore.setState((s) => ({ info: s.info ? { ...s.info, volume } : null, overview: s.overview ? { ...s.overview, volume } : null })))]);
      useStore.setState((s) => ({ cleanupBusy: false, dataVersion: s.dataVersion + (s.dataVersion === versionBeforeRefresh ? 1 : 0) }));
    }
  };
  const simulatorNote = mode === 'delete' ? 'Simulator apps and their data are lost; shared OS runtimes remain installed.' : 'Keep Xcode, Simulator and agents closed until you restore or purge. Restoring cannot replace a device recreated at the same path.';
  const current = progress ? list[Math.min(progress.index, list.length - 1)] : undefined;
  return (
    <Modal open={cleanModalOpen} onClose={close} locked={busy} width={520}
      title={result ? (result.failures.length ? 'Cleanup finished with problems' : mode === 'delete' ? 'Deleted' : 'Moved to quarantine')
        : busy ? `${m.doing}…` : `Clean up ${formatBytes(total)}`}
      subtitle={result ? `${result.paths.length} of ${list.length} item${list.length === 1 ? '' : 's'} ${mode === 'delete' ? 'deleted' : 'moved'}`
        : busy ? (finishing ? 'Finishing up…' : progress && current ? `${Math.min(progress.index + 1, progress.count)} of ${progress.count} · ${current.name}` : 'Starting…')
        : `${staged.length} item${staged.length === 1 ? '' : 's'}`}
      footer={result ? <>
        {mode === 'quarantine' && <button className="btn" onClick={() => { close(); setPage('quarantine'); }}>Open quarantine</button>}
        <button className="btn btn-primary" onClick={close}>Done</button>
      </> : busy ? (
        <span className="mr-auto text-[12px]" style={{ color: 'var(--muted)' }}>Keep Disko open until this finishes.</span>
      ) : <>
        <button className="btn btn-ghost" onClick={close}>Cancel</button>
        <button className="btn btn-primary" style={mode === 'delete' ? { background: 'var(--danger)', borderColor: 'var(--danger)', color: '#fff' } : undefined} onClick={run} disabled={staged.length === 0}>
          {m.verb} {formatBytes(total)}
        </button>
      </>}>
      {result ? <div>
        <div className="flex items-baseline gap-2">
          <span className="tnum text-[32px] font-medium">{formatBytes(result.bytes)}</span>
          <span className="text-[13px]" style={{ color: 'var(--muted)' }}>{mode === 'delete' ? 'freed' : 'quarantined'}</span>
        </div>
        <p className="text-[12px] mt-1" style={{ color: 'var(--muted)' }}>{mode === 'delete' ? 'Free space may differ slightly because of APFS snapshots and shared blocks.' : 'Restore from Quarantine, or purge there to free the space.'}</p>
        {hasSimulator && <p className="text-[12px] mt-2" style={{ color: 'var(--warn)' }}>{simulatorNote}</p>}
        {result.failures.map((f) => <p key={f.path} className="text-[12px] mt-3 break-all selectable" style={{ color: 'var(--warn)' }}>{shortPath(f.path, info?.home ?? '')}: {f.error}</p>)}
      </div> : <div>
        {busy ? <div aria-live="polite">
          <div className="flex items-baseline justify-between">
            <span className="tnum text-[13px]">{formatBytes(progress?.done_bytes ?? 0)} <span style={{ color: 'var(--muted)' }}>of {formatBytes(progress?.total_bytes ?? listTotal)}</span></span>
            <span className="tnum text-[12px]" style={{ color: 'var(--muted)' }}>{Math.round(fraction * 100)}%</span>
          </div>
          <div className="h-1.5 rounded-full mt-2 overflow-hidden" style={{ background: 'var(--bg-3)' }} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)}>
            <motion.div className={`h-full rounded-full ${finishing ? 'shimmer' : ''}`} style={{ background: finishing ? undefined : mode === 'delete' ? 'var(--danger)' : 'var(--text)' }}
              initial={false} animate={{ width: `${Math.max(fraction, 0.01) * 100}%` }} transition={{ ease: 'easeOut', duration: 0.35 }} />
          </div>
        </div> : <>
          <div className="grid grid-cols-2 gap-1 p-1 rounded-lg" style={{ background: 'var(--bg-3)' }} role="group" aria-label="Cleanup method">
            {(['quarantine', 'delete'] as const).map((value) => {
              const active = mode === value;
              const Icon = MODES[value].icon;
              return <button key={value} aria-pressed={active} onClick={() => useStore.setState({ cleanupMode: value })}
                className="relative h-8 rounded-md text-[12.5px] font-medium transition-colors" style={{ color: active ? (value === 'delete' ? 'var(--danger)' : 'var(--text)') : 'var(--muted)' }}>
                {active && <motion.span layoutId="clean-mode-bg" className="absolute inset-0 rounded-md" style={{ background: 'var(--bg-2)', boxShadow: '0 1px 3px rgba(0,0,0,0.2)' }} transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
                <span className="relative flex items-center justify-center gap-1.5"><Icon className="w-3.5 h-3.5" />{MODES[value].label}</span>
              </button>;
            })}
          </div>
          <p className="text-[12px] mt-2" style={{ color: mode === 'delete' ? 'var(--danger)' : 'var(--muted)' }}>{m.note}</p>
        </>}

        <ul className="mt-4 -mx-2 max-h-60 overflow-y-auto">
          {list.map((s, i) => {
            const state = !busy || !progress ? 'idle' : i < progress.finished ? 'done' : i === progress.index ? 'active' : 'pending';
            return <li key={s.path} className="flex items-center gap-3 px-2 py-1.5 rounded-md transition-opacity" title={[s.path, s.reason].filter(Boolean).join('\n')} style={{ opacity: state === 'pending' ? 0.45 : 1 }}>
              <span className="w-3.5 flex items-center justify-center flex-shrink-0">
                {state === 'done' ? <Check className="w-3.5 h-3.5" style={{ color: 'var(--ok)' }} />
                  : state === 'active' ? <Spinner size={12} />
                  : <span className="w-2 h-2 rounded-full" style={{ background: catMeta(s.category).color }} />}
              </span>
              <div className="flex-1 min-w-0">
                <div className="text-[12.5px] truncate">{s.name}</div>
                {(advanced || s.reason) && <div className={`text-[11px] truncate ${advanced ? 'mono' : ''}`} style={{ color: 'var(--dim)' }}>{advanced ? shortPath(s.path, info?.home ?? '') : s.reason}</div>}
              </div>
              <span className="tnum text-[12px] flex-shrink-0" style={{ color: 'var(--muted)' }}>{s.total ? formatBytes(s.total) : '—'}</span>
            </li>;
          })}
        </ul>

        {!busy && <>
          {hasSimulator && <p className="text-[12px] mt-3" style={{ color: 'var(--warn)' }}>{simulatorNote}</p>}
          <p className="text-[11.5px] mt-3" style={{ color: 'var(--dim)' }}>Quit apps and stop builds that use these files first.</p>
        </>}
        {error && <p role="alert" className="text-[12px] mt-3 selectable" style={{ color: 'var(--danger)' }}>{error}</p>}
      </div>}
    </Modal>
  );
};
