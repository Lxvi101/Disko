import React, { useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence, animate } from 'framer-motion';
import { create } from 'zustand';
import { ArrowRight, Check, TriangleAlert } from 'lucide-react';
import { useStore } from '../../store';
import { api, errorText, onQuickProgress, onQuickScan } from '../../lib/api';
import { formatBytes } from '../../lib/format';
import { clusterGroups } from '../../lib/programs';
import { ClusterBrowser, selectedBytes } from '../ClusterBrowser';
import type { QuickCleanOutcome, QuickGroup, QuickProgress, QuickReport, QuickScanEvent } from '../../types';

type Phase = 'idle' | 'results' | 'cleaning' | 'done';
type Step = { label: string; done: number; count: number };

/** Kept outside the page so results survive switching tabs. */
const useQuick = create<{
  phase: Phase; report: QuickReport | null; selected: Set<string>; progress: QuickProgress | null; outcome: QuickCleanOutcome | null; error: string | null;
  measuring: boolean; step: Step | null;
  set: (p: Partial<{ phase: Phase; report: QuickReport | null; selected: Set<string>; progress: QuickProgress | null; outcome: QuickCleanOutcome | null; error: string | null; measuring: boolean; step: Step | null }>) => void;
}>((set) => ({ phase: 'idle', report: null, selected: new Set(), progress: null, outcome: null, error: null, measuring: false, step: null, set: (p) => set(p) }));

const MIN_ITEM = 512 * 1024;
const ticked = (groups: QuickGroup[]) => groups.flatMap((g) => g.items.filter((i) => i.preselect && !i.blocked).map((i) => i.id));

/** Discovery streams in: groups as soon as they're found, then sizes. Fold each event into the report. */
function onEvent(e: QuickScanEvent) {
  const q = useQuick.getState();
  if (!q.measuring || !q.report) return;
  if (e.kind === 'step') q.set({ step: e });
  else if (e.kind === 'found') {
    q.set({ report: { ...q.report, groups: [...q.report.groups, ...e.groups] }, selected: new Set([...q.selected, ...ticked(e.groups)]) });
  } else if (e.kind === 'sized') {
    const sizes = new Map(e.sizes);
    const groups = q.report.groups.map((g) => {
      if (!g.items.some((i) => sizes.has(i.id))) return g;
      const items = g.items.map((i) => (sizes.has(i.id) ? { ...i, total: sizes.get(i.id)!, measured: true } : i));
      return { ...g, items, total: items.reduce((n, i) => n + i.total, 0) };
    });
    q.set({ report: { ...q.report, groups } });
  }
}

let listening = false;
function listen() {
  if (listening) return;
  listening = true;
  onQuickProgress((p) => useQuick.getState().set({ progress: p })).catch(() => { listening = false; });
  onQuickScan(onEvent).catch(() => { listening = false; });
}

async function runScan() {
  useQuick.getState().set({ phase: 'results', report: { generated_at: 0, groups: [], notes: [] }, selected: new Set(), measuring: true, step: null, error: null, outcome: null });
  try {
    const report = await api.quickScan();
    // Keep whatever the user already unticked or ticked while sizes were still coming in.
    const ids = new Set(report.groups.flatMap((g) => g.items.map((i) => i.id)));
    const selected = new Set([...useQuick.getState().selected].filter((id) => ids.has(id)));
    useQuick.getState().set({ report, selected, measuring: false, step: null });
  } catch (e) {
    const has = !!useQuick.getState().report?.groups.length;
    useQuick.getState().set({ phase: has ? 'results' : 'idle', measuring: false, error: errorText(e) });
  }
}

/** While measuring, tiny folders vanish once their size is known and the rest sort by size. */
function visible(groups: QuickGroup[]): QuickGroup[] {
  return groups.map((g) => {
    const items = g.items.filter((i) => i.measured === false || i.total >= MIN_ITEM)
      .sort((a, b) => Number(a.measured === false) - Number(b.measured === false) || b.total - a.total);
    return { ...g, items };
  }).filter((g) => g.items.length && (g.items.some((i) => i.measured === false) || g.total >= 4 * MIN_ITEM));
}

async function runClean() {
  const { selected } = useQuick.getState();
  if (!selected.size) return;
  useQuick.getState().set({ phase: 'cleaning', progress: null, error: null });
  useStore.setState({ cleanupBusy: true });
  try {
    const outcome = await api.quickClean([...selected]);
    useQuick.getState().set({ phase: 'done', outcome });
    useStore.setState((s) => ({
      overview: s.overview ? { ...s.overview, volume: outcome.after } : s.overview,
      info: s.info ? { ...s.info, volume: outcome.after } : s.info,
    }));
    const st = useStore.getState();
    st.refreshCurrent();
    st.reloadData();
  } catch (e) {
    useQuick.getState().set({ phase: 'results', error: errorText(e) });
  } finally {
    useStore.setState({ cleanupBusy: false });
  }
}

function useCountUp(value: number, duration = 0.9) {
  const [shown, setShown] = useState(value);
  const from = useRef(value);
  useEffect(() => {
    const c = animate(from.current, value, { duration, ease: [0.16, 1, 0.3, 1], onUpdate: (v) => { from.current = v; setShown(v); } });
    return () => c.stop();
  }, [value, duration]);
  return shown;
}

export const QuickCleanPage: React.FC = () => {
  const { phase, report, selected, progress, outcome, error, measuring, step, set } = useQuick();
  useEffect(listen, []);
  const clusters = useMemo(() => clusterGroups(visible(report?.groups ?? [])), [report]);

  return (
    <div className="h-full overflow-y-auto quick-page">
      <AnimatePresence mode="wait" initial={false}>
        {phase === 'idle' && <Idle key="idle" error={error} />}
        {phase === 'results' && report && (
          <Results key="results" clusters={clusters} notes={report.notes} selected={selected} error={error} measuring={measuring} step={step} onChange={(s) => set({ selected: s })} />
        )}
        {phase === 'cleaning' && <Cleaning key="clean" progress={progress} total={selectedBytes(clusters, selected).bytes} />}
        {phase === 'done' && outcome && <Done key="done" outcome={outcome} />}
      </AnimatePresence>
    </div>
  );
};

// ---------------------------------------------------------------------------

/** A thin progress ring. `value` is 0–1; leave it undefined for an idle ring. */
function Ring({ value, spinning, children, size = 200 }: { value?: number; spinning?: boolean; children?: React.ReactNode; size?: number }) {
  const r = size / 2 - 4;
  const c = 2 * Math.PI * r;
  return (
    <div className="usage-ring" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} style={{ transform: 'rotate(-90deg)' }} className={spinning && value === undefined ? 'animate-spin-slow' : ''}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--line-2)" strokeWidth={3} />
        <motion.circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--text)" strokeWidth={3} strokeLinecap="round" strokeDasharray={c}
          initial={false} animate={{ strokeDashoffset: c * (1 - Math.min(1, Math.max(0, value ?? (spinning ? 0.18 : 0)))) }} transition={{ type: 'spring', stiffness: 60, damping: 20 }} />
      </svg>
      <div className="usage-ring-core">{children}</div>
    </div>
  );
}

const page = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: -6 }, transition: { duration: 0.18 } };

function Idle({ error }: { error: string | null }) {
  const vol = useStore((s) => s.overview?.volume ?? s.info?.volume);
  const used = vol?.total ? 1 - vol.available / vol.total : 0;
  return (
    <motion.div {...page} className="quick-hero">
      <Ring value={used}>
        <span className="text-[30px] font-semibold tracking-tight tnum">{vol?.total ? formatBytes(vol.available) : '—'}</span>
        <span className="text-[12px]" style={{ color: 'var(--muted)' }}>free{vol?.total ? ` of ${formatBytes(vol.total)}` : ''}</span>
      </Ring>
      <h1 className="text-[28px] font-semibold tracking-tight mt-9">Quick Clean</h1>
      <p className="text-[13.5px] mt-2 max-w-[400px] text-center leading-relaxed" style={{ color: 'var(--muted)' }}>
        Finds build folders, package caches, Xcode data, old installers and the Trash. Takes a few seconds. Anything in use is skipped.
      </p>
      <button className="btn btn-primary btn-lg mt-7" onClick={runScan}>Scan</button>
      {error && <p role="alert" className="text-[12.5px] mt-4" style={{ color: 'var(--danger)' }}>{error}</p>}
    </motion.div>
  );
}

function Results({ clusters, notes, selected, error, measuring, step, onChange }: { clusters: ReturnType<typeof clusterGroups>; notes: string[]; selected: Set<string>; error: string | null; measuring: boolean; step: Step | null; onChange: (s: Set<string>) => void }) {
  const total = clusters.reduce((n, c) => n + c.total, 0);
  const sel = selectedBytes(clusters, selected);
  const shownTotal = useCountUp(total, measuring ? 0.4 : 0.8);
  const shownSel = useCountUp(sel.bytes, 0.4);
  if (!clusters.length && !measuring) {
    return (
      <motion.div {...page} className="quick-hero">
        <Ring value={1}><Check className="w-9 h-9" strokeWidth={1.5} /></Ring>
        <h1 className="text-[24px] font-semibold tracking-tight mt-8">Nothing to clean</h1>
        <p className="text-[13.5px] mt-2" style={{ color: 'var(--muted)' }}>Your Mac is already tidy.</p>
        <button className="btn mt-6" onClick={runScan}>Scan again</button>
      </motion.div>
    );
  }
  return (
    <motion.div {...page} className="max-w-[1180px] mx-auto px-10 pt-8 pb-10">
      <header className="flex items-end gap-6 mb-7">
        <div className="flex-1 min-w-0">
          <p className="text-[12.5px]" style={{ color: 'var(--muted)' }}>{measuring ? 'Measuring…' : `Found in ${clusters.length} programs`}</p>
          <h1 className="text-[40px] font-semibold tracking-tight leading-none mt-1.5 tnum">{formatBytes(shownTotal)}</h1>
          {measuring ? (
            <div className="mt-3 max-w-[320px]">
              <div className="scan-bar"><motion.span animate={{ width: `${step?.count ? Math.max(3, (step.done / step.count) * 100) : 3}%` }} transition={{ type: 'spring', stiffness: 80, damping: 20 }} /></div>
              <p className="text-[11.5px] mt-1.5 tnum" style={{ color: 'var(--dim)' }}>{step?.label ?? 'Looking around'}…</p>
            </div>
          ) : (
            <p className="text-[12.5px] mt-2" style={{ color: 'var(--muted)' }}>Selected items aren’t in use and rebuild themselves.</p>
          )}
        </div>
        <div className="flex flex-col items-end gap-2">
          <button className="btn btn-primary btn-lg" disabled={!sel.count || measuring} onClick={runClean}>
            Clean <span className="tnum">{formatBytes(shownSel)}</span>
          </button>
          <span className="text-[11px]" style={{ color: 'var(--dim)' }}>
            {measuring ? 'You can select items while it measures' : <>{sel.count} {sel.count === 1 ? 'item' : 'items'} · deleted, not quarantined · <button className="underline underline-offset-2" onClick={runScan}>rescan</button></>}
          </span>
        </div>
      </header>
      {error && <p role="alert" className="text-[12.5px] mb-3" style={{ color: 'var(--danger)' }}>{error}</p>}
      {notes.map((n) => <p key={n} className="quick-note"><TriangleAlert className="w-3.5 h-3.5 flex-shrink-0" /> {n}</p>)}
      {clusters.length ? <ClusterBrowser clusters={clusters} selected={selected} onChange={onChange} /> : <Skeleton />}
    </motion.div>
  );
}

/** Placeholder rows for the first few hundred milliseconds. */
function Skeleton() {
  return (
    <div className="cluster-browser">
      <div className="cluster-list">{[0, 1, 2, 3].map((i) => <div key={i} className="cluster-card skeleton" style={{ animationDelay: `${i * 120}ms` }} />)}</div>
      <div className="cluster-detail skeleton" style={{ height: 320 }} />
    </div>
  );
}

function Cleaning({ progress, total }: { progress: QuickProgress | null; total: number }) {
  const freed = useCountUp(Math.min(total, progress?.freed_bytes ?? 0), 0.6);
  const frac = progress?.count ? progress.index / progress.count : 0;
  return (
    <motion.div {...page} className="quick-hero">
      <Ring value={frac}>
        <span className="text-[26px] font-semibold tracking-tight tnum">{formatBytes(freed)}</span>
        <span className="text-[12px]" style={{ color: 'var(--muted)' }}>of {formatBytes(total)}</span>
      </Ring>
      <h1 className="text-[22px] font-semibold tracking-tight mt-8">Cleaning</h1>
      <p className="text-[13px] mt-2 max-w-[460px] truncate" style={{ color: 'var(--muted)' }}>{progress?.label ?? '…'}</p>
    </motion.div>
  );
}

function Done({ outcome }: { outcome: QuickCleanOutcome }) {
  const freed = useCountUp(outcome.result.freed_bytes, 1.2);
  const { before, after } = outcome;
  const gained = Math.max(0, after.available - before.available);
  const fails = outcome.result.failures;
  const [showFails, setShowFails] = useState(false);
  const goJunk = () => useStore.getState().setPage('junk');
  return (
    <motion.div {...page} className="quick-hero">
      <Ring value={1}>
        <motion.span initial={{ scale: 0.6, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} transition={{ type: 'spring', stiffness: 300, damping: 18 }}><Check className="w-10 h-10" strokeWidth={1.5} /></motion.span>
      </Ring>
      <p className="text-[12.5px] mt-8" style={{ color: 'var(--muted)' }}>Freed up</p>
      <h1 className="text-[46px] font-semibold tracking-tight leading-none mt-1 tnum">{formatBytes(freed)}</h1>
      {before.total > 0 && (
        <div className="w-[360px] mt-7">
          <FreeBar label="Before" vol={before} />
          <FreeBar label="Now" vol={after} />
          <p className="text-[11.5px] mt-2 text-center" style={{ color: 'var(--dim)' }}>
            {gained > 0 ? `macOS reports ${formatBytes(gained)} more free space.` : 'macOS may take a minute to report the new free space.'}
          </p>
        </div>
      )}
      {fails.length > 0 && (
        <div className="mt-5 text-center">
          <button className="text-[12.5px] underline underline-offset-2" style={{ color: 'var(--muted)' }} onClick={() => setShowFails(!showFails)}>{fails.length} {fails.length === 1 ? 'item was' : 'items were'} skipped</button>
          {showFails && (
            <ul className="quick-fails selectable">
              {fails.map((f) => <li key={f.label + f.error}><strong>{f.label}</strong>: {f.error}</li>)}
            </ul>
          )}
        </div>
      )}
      <div className="flex gap-2 mt-8">
        <button className="btn" onClick={runScan}>Scan again</button>
        <button className="btn btn-primary" onClick={goJunk}>Find more in Junk <ArrowRight className="w-3.5 h-3.5" /></button>
      </div>
    </motion.div>
  );
}

function FreeBar({ label, vol }: { label: string; vol: { total: number; available: number } }) {
  const free = vol.available / Math.max(1, vol.total);
  return (
    <div className="flex items-center gap-3 mt-2">
      <span className="text-[11.5px] w-12" style={{ color: 'var(--muted)' }}>{label}</span>
      <div className="disk-bar flex-1"><motion.span initial={{ width: 0 }} animate={{ width: `${(1 - free) * 100}%` }} transition={{ duration: 1, ease: [0.16, 1, 0.3, 1] }} /></div>
      <span className="tnum text-[12px] w-[86px] text-right">{formatBytes(vol.available)} free</span>
    </div>
  );
}
