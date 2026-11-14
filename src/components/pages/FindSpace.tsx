import React, { useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence, animate } from 'framer-motion';
import { create } from 'zustand';
import { ArrowRight, ArrowUp, Check, RefreshCw } from 'lucide-react';
import { useStore } from '../../store';
import { GlassFolder } from '../ui';
import { api, errorText, onQuickProgress, onQuickScan } from '../../lib/api';
import { formatBytes } from '../../lib/format';
import type { QuickCleanOutcome, QuickGroup, QuickItem, QuickProgress, QuickReport, QuickScanEvent } from '../../types';

/*
 * Find space: ask, see what Disko found and why, pick, clean, see what really happened.
 * Discovery and cleanup are Quick Clean's (src-tauri/src/quick_clean.rs); this page is the flow around them.
 */

type Phase = 'start' | 'finding' | 'plan' | 'cleaning' | 'done';
type Step = { label: string; done: number; count: number };
/** What the user asked for, applied to the selection once sizes are known. */
type Intent = 'safe' | 'rebuilds' | 'fifty';

const MIN_ITEM = 512 * 1024;
const GB = 1e9;

type FindState = {
  phase: Phase; report: QuickReport | null; selected: Set<string>; progress: QuickProgress | null; outcome: QuickCleanOutcome | null;
  error: string | null; step: Step | null; intent: Intent; asked: string; confirming: boolean; startedAt: number;
};
/** Kept outside the page so a plan survives switching tabs. */
const useFind = create<FindState & { set: (p: Partial<FindState>) => void }>((set) => ({
  phase: 'start', report: null, selected: new Set(), progress: null, outcome: null, error: null, step: null, intent: 'safe', asked: '', confirming: false, startedAt: 0,
  set: (p) => set(p),
}));

// ---------------------------------------------------------------------------
// What each finding is

const PROJECT = new Set(['rust-target', 'node-modules', 'web-build', 'swift-build', 'python-venv', 'python-cache', 'pods', 'gradle-project']);
const REINSTALL = new Set(['node-modules', 'python-venv', 'pods']);

type Section = 'build' | 'cache' | 'system' | 'tool';
const SECTIONS: Record<Section, { title: string; color: string }> = {
  build: { title: 'Build leftovers', color: '#5b8cff' },
  cache: { title: 'Caches & downloads', color: '#2fbf86' },
  system: { title: 'Trash, logs & installers', color: '#f5a524' },
  tool: { title: 'Cleaned by their own tool', color: '#a78bfa' },
};
const ORDER: Section[] = ['build', 'cache', 'system', 'tool'];

function sectionOf(g: QuickGroup): Section {
  if (g.method === 'command') return 'tool';
  if (PROJECT.has(g.id) || g.program === 'Xcode') return 'build';
  if (g.program === 'macOS') return 'system';
  return 'cache';
}

type Pill = { text: string; tone: 'safe' | 'check' | 'busy' | 'tool' | 'trash' };
function pillOf(g: QuickGroup, i: QuickItem): Pill {
  if (i.blocked) return { text: 'In use', tone: 'busy' };
  if (g.method === 'command') return { text: 'Tool decides', tone: 'tool' };
  if (g.id === 'trash') return { text: 'In Trash', tone: 'trash' };
  if (REINSTALL.has(g.id)) return { text: 'Reinstall', tone: 'check' };
  if (!i.preselect) return { text: PROJECT.has(g.id) ? 'Built today' : 'Look first', tone: 'check' };
  return { text: sectionOf(g) === 'build' ? 'Rebuilds' : 'Downloads again', tone: 'safe' };
}

function whyOf(g: QuickGroup, i: QuickItem): string {
  if (i.blocked) return i.blocked;
  const where = i.detail && i.detail !== i.label ? i.detail : '';
  return [g.title !== i.label ? g.title : '', where].filter(Boolean).join(' · ') || g.blurb;
}

type Row = { g: QuickGroup; i: QuickItem; section: Section };
function rowsOf(report: QuickReport | null): Row[] {
  if (!report) return [];
  return report.groups.flatMap((g) => g.items.filter((i) => i.measured === false || i.total >= MIN_ITEM).map((i) => ({ g, i, section: sectionOf(g) })));
}

function tally(rows: Row[], selected: Set<string>) {
  let now = 0, tool = 0, waiting = 0, count = 0;
  const waitingRows: Row[] = [];
  for (const r of rows) {
    if (!selected.has(r.i.id)) continue;
    count++;
    if (r.i.blocked) { waiting += r.i.total; waitingRows.push(r); } else if (r.g.method === 'command') tool += r.i.total; else now += r.i.total;
  }
  return { now, tool, waiting, count, waitingRows };
}

function selectionFor(intent: Intent, rows: Row[]): Set<string> {
  const ok = rows.filter((r) => !r.i.blocked);
  if (intent === 'rebuilds') return new Set(ok.filter((r) => r.i.preselect && pillOf(r.g, r.i).tone === 'safe' && r.section === 'build').map((r) => r.i.id));
  if (intent === 'fifty') {
    // The safest things first, biggest first, until 50 GB.
    const rank = (r: Row) => (r.i.preselect ? 0 : 1) + (r.g.method === 'command' ? 1 : 0);
    const out = new Set<string>();
    let sum = 0;
    for (const r of [...ok].sort((a, b) => rank(a) - rank(b) || b.i.total - a.i.total)) {
      if (sum >= 50 * GB) break;
      if (pillOf(r.g, r.i).tone === 'check' && rank(r) > 0 && sum + r.i.total > 80 * GB) continue;
      out.add(r.i.id); sum += r.i.total;
    }
    return out;
  }
  return new Set(ok.filter((r) => r.i.preselect).map((r) => r.i.id));
}

// ---------------------------------------------------------------------------
// Talking to the backend

function onEvent(e: QuickScanEvent) {
  const q = useFind.getState();
  if (q.phase !== 'finding' || !q.report) return;
  if (e.kind === 'step') q.set({ step: e });
  else if (e.kind === 'found') {
    // Tick the safe ones as they arrive; a narrower request ("only rebuilds", "50 GB") is applied once sizes are in.
    const add = q.intent === 'safe' ? e.groups.flatMap((g) => g.items.filter((i) => i.preselect && !i.blocked).map((i) => i.id)) : [];
    q.set({ report: { ...q.report, groups: [...q.report.groups, ...e.groups] }, selected: new Set([...q.selected, ...add]) });
  }
  else if (e.kind === 'sized') {
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
  onQuickProgress((p) => useFind.getState().set({ progress: p })).catch(() => { listening = false; });
  onQuickScan(onEvent).catch(() => { listening = false; });
}

async function find(intent: Intent, asked: string, keep?: Set<string>) {
  useFind.getState().set({ phase: 'finding', intent, asked, report: { generated_at: 0, groups: [], notes: [] }, selected: new Set(), step: null, error: null, outcome: null, confirming: false, startedAt: Date.now() });
  try {
    const report = await api.quickScan();
    const rows = rowsOf(report);
    let selected = selectionFor(intent, rows);
    // "Check again" keeps what was picked, including things that were in use and may be free now.
    if (keep) { const ids = new Set(rows.map((r) => r.i.id)); selected = new Set([...keep].filter((id) => ids.has(id))); }
    useFind.getState().set({ phase: 'plan', report, selected, step: null });
  } catch (e) {
    useFind.getState().set({ phase: 'start', error: errorText(e) });
  }
}

async function clean() {
  const { selected } = useFind.getState();
  if (!selected.size) return;
  useFind.getState().set({ phase: 'cleaning', progress: null, error: null, confirming: false });
  useStore.setState({ cleanupBusy: true });
  try {
    const outcome = await api.quickClean([...selected]);
    useFind.getState().set({ phase: 'done', outcome });
    useStore.setState((s) => ({
      overview: s.overview ? { ...s.overview, volume: outcome.after } : s.overview,
      info: s.info ? { ...s.info, volume: outcome.after } : s.info,
    }));
    const st = useStore.getState();
    st.refreshCurrent();
    st.reloadData();
  } catch (e) {
    useFind.getState().set({ phase: 'plan', error: errorText(e) });
  } finally {
    useStore.setState({ cleanupBusy: false });
  }
}

/** Free text that is about making room runs the sweep; anything else is a question for the assistant. */
function ask(text: string) {
  const t = text.trim();
  const info = useStore.getState().info;
  const cleanup = !t || /\b(space|room|clean|clear|free|junk|full|delete|remove|tidy|trash|cache|build|gb)\b/i.test(t);
  if (cleanup || !info?.codex_version) {
    const intent: Intent = /\b50\b|fifty/i.test(t) ? 'fifty' : /rebuild/i.test(t) ? 'rebuilds' : 'safe';
    find(intent, t);
  } else {
    useStore.getState().askAssistant(t);
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

// ---------------------------------------------------------------------------
// Page

export const QuickCleanPage: React.FC = () => {
  const phase = useFind((s) => s.phase);
  const report = useFind((s) => s.report);
  const selected = useFind((s) => s.selected);
  useEffect(listen, []);
  const rows = useMemo(() => rowsOf(report), [report]);

  // Let the sidebar preview the disk after this plan.
  const now = phase === 'plan' ? tally(rows, selected).now : 0;
  useEffect(() => { useStore.setState({ planBytes: now }); }, [now]);
  useEffect(() => () => useStore.setState({ planBytes: 0 }), []);

  return (
    <div className="fs-page">
      <AnimatePresence mode="wait" initial={false}>
        {phase === 'start' && <Start key="start" />}
        {(phase === 'finding' || phase === 'plan') && <Plan key="plan" rows={rows} />}
        {phase === 'cleaning' && <Cleaning key="cleaning" />}
        {phase === 'done' && <Done key="done" />}
      </AnimatePresence>
    </div>
  );
};

const fade = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: -6 }, transition: { duration: 0.2 } };

// ---------------------------------------------------------------------------
// Pieces


const CHIPS: { label: string; run: () => void }[] = [
  { label: 'Everything that’s safe', run: () => find('safe', '') },
  { label: 'Find 50 GB', run: () => find('fifty', 'Find 50 GB') },
  { label: 'Only stuff that rebuilds', run: () => find('rebuilds', 'Only stuff that rebuilds') },
  { label: 'What’s taking space?', run: () => { const st = useStore.getState(); st.setPage('explore'); } },
];

const AskBox: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const asked = useFind((s) => s.asked);
  const phase = useFind((s) => s.phase);
  const [text, setText] = useState(asked);
  useEffect(() => setText(asked), [asked]);
  const codex = useStore((s) => !!s.info?.codex_version);
  const busy = phase === 'finding';
  return (
    <form className="fs-ask" onSubmit={(e) => { e.preventDefault(); if (!busy) ask(text); }}>
      <div className="fs-ask-row">
        <input
          className="fs-ask-input"
          value={text}
          autoFocus={autoFocus}
          onChange={(e) => setText(e.target.value)}
          placeholder={codex ? 'What should I clear? Or ask anything about your disk' : 'What should I clear? Try “find 50 GB”'}
          aria-label="What should Disko clear?"
        />
        <button className="fs-send" type="submit" disabled={busy} aria-label="Find space">
          {busy ? <RefreshCw className="w-4 h-4 animate-spin" /> : <ArrowUp className="w-4 h-4" strokeWidth={2.4} />}
        </button>
      </div>
      <div className="fs-chips">
        {CHIPS.map((c) => <button key={c.label} type="button" className="fs-chip" disabled={busy} onClick={c.run}>{c.label}</button>)}
      </div>
    </form>
  );
};

function Start() {
  const vol = useStore((s) => s.overview?.volume ?? s.info?.volume);
  const error = useFind((s) => s.error);
  const used = vol?.total ? 1 - vol.available / vol.total : 0;
  const headline = !vol?.total ? 'Let’s make some room' : used > 0.9 ? `Your disk is ${Math.round(used * 100)}% full` : `${formatBytes(vol.available)} free`;
  return (
    <motion.section {...fade} className="fs-start">
      <div className="fs-hero">
        <button className="fs-hero-folder" onClick={() => find('safe', '')} aria-label="Find space"><GlassFolder big /></button>
        <h1>{headline}</h1>
        <p>Tell me what you want, or just press return. I’ll look through your projects, caches, Xcode, Docker and the Trash, tell you what each thing is, and skip anything that’s in use.</p>
        {error && <p role="alert" className="fs-error">{error}</p>}
      </div>
      <AskBox autoFocus />
    </motion.section>
  );
}

const PillTag: React.FC<{ pill: Pill }> = ({ pill }) => <span className={`fs-pill fs-pill-${pill.tone}`}>{pill.text}</span>;

const RowView: React.FC<{ r: Row; on: boolean; onToggle: () => void; fresh: boolean }> = ({ r, on, onToggle, fresh }) => {
  const sizing = r.i.measured === false;
  return (
    <motion.button
      layout="position"
      initial={fresh ? { opacity: 0, y: 10, scale: 0.98 } : false}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ type: 'spring', stiffness: 420, damping: 30 }}
      className="fs-row"
      data-on={on}
      onClick={onToggle}
      title={`${r.g.blurb} ${r.g.after}`}
      aria-pressed={on}
    >
      <span className="fs-check" aria-hidden="true">{on && <Check className="w-3 h-3" strokeWidth={3} />}</span>
      <GlassFolder color={SECTIONS[r.section].color} />
      <span className="fs-row-text">
        <span className="fs-row-title">{r.i.label}</span>
        <span className="fs-row-why">{whyOf(r.g, r.i)}</span>
      </span>
      <PillTag pill={pillOf(r.g, r.i)} />
      <span className="fs-row-size tnum">{sizing ? <span className="fs-shim" /> : (r.g.method === 'command' ? 'up to ' : '') + formatBytes(r.i.total)}</span>
    </motion.button>
  );
};

const SHOW = 6;

function Plan({ rows }: { rows: Row[] }) {
  const { phase, selected, set, step, error, startedAt, confirming } = useFind();
  const measuring = phase === 'finding';
  const [open, setOpen] = useState<Set<Section>>(new Set());
  const seen = useRef(new Set<string>());
  const t = tally(rows, selected);
  const toggle = (id: string) => { const s = new Set(selected); s.has(id) ? s.delete(id) : s.add(id); set({ selected: s }); };
  const bySection = ORDER.map((sec) => ({ sec, rows: rows.filter((r) => r.section === sec).sort((a, b) => Number(a.i.measured === false) - Number(b.i.measured === false) || b.i.total - a.i.total) })).filter((s) => s.rows.length);
  const fresh = (id: string) => { if (seen.current.has(id)) return false; seen.current.add(id); return measuring; };

  return (
    <motion.div {...fade} className="fs-plan">
      <section className="fs-feed">
        <AskBox />
        <div className="fs-say">
          <GlassFolder mood={measuring ? 'breathe' : 'wobble'} />
          <p>{measuring ? <Looking rows={rows} /> : <Summary rows={rows} />}</p>
        </div>
        {error && <p role="alert" className="fs-error">{error}</p>}
        <div className="fs-list">
          {bySection.map(({ sec, rows: rs }) => {
            const all = open.has(sec) || measuring;
            const shown = all ? rs : rs.slice(0, SHOW);
            const rest = rs.slice(SHOW);
            const total = rs.reduce((n, r) => n + r.i.total, 0);
            const picked = rs.filter((r) => selected.has(r.i.id));
            return (
              <div key={sec} className="fs-section">
                <div className="fs-section-head">
                  <span>{SECTIONS[sec].title}</span>
                  {!measuring && (
                    <button className="fs-section-all" onClick={() => {
                      const s = new Set(selected);
                      const pickable = rs.filter((r) => !r.i.blocked);
                      const allOn = pickable.every((r) => s.has(r.i.id));
                      pickable.forEach((r) => (allOn ? s.delete(r.i.id) : s.add(r.i.id)));
                      set({ selected: s });
                    }}>{picked.length ? `${picked.length} picked` : 'Pick all'}</button>
                  )}
                  <span className="tnum">{formatBytes(total)}</span>
                </div>
                {shown.map((r) => <RowView key={r.i.id} r={r} on={selected.has(r.i.id)} onToggle={() => toggle(r.i.id)} fresh={fresh(r.i.id)} />)}
                {!all && rest.length > 0 && (
                  <button className="fs-more" onClick={() => setOpen(new Set([...open, sec]))}>
                    {rest.length} smaller {rest.length === 1 ? 'one' : 'ones'} · {formatBytes(rest.reduce((n, r) => n + r.i.total, 0))}
                  </button>
                )}
              </div>
            );
          })}
          {measuring && rows.length === 0 && [0, 1, 2].map((i) => <div key={i} className="fs-row fs-row-skel" style={{ animationDelay: `${i * 120}ms` }} />)}
        </div>
      </section>
      <aside className="fs-side">
        {measuring ? <Checklist step={step} startedAt={startedAt} rows={rows} /> : <PlanPanel t={t} />}
      </aside>
      <AnimatePresence>{confirming && <Confirm t={t} rows={rows} />}</AnimatePresence>
    </motion.div>
  );
}

function Looking({ rows }: { rows: Row[] }) {
  const sum = useCountUp(rows.reduce((n, r) => n + r.i.total, 0), 0.4);
  return <>Looking through your disk… <b className="tnum">{formatBytes(sum)}</b> so far.</>;
}

function Summary({ rows }: { rows: Row[] }) {
  const free = rows.filter((r) => !r.i.blocked);
  const found = free.reduce((n, r) => n + r.i.total, 0);
  if (!rows.length) return <>All clean. Nothing in your projects, caches or Trash is worth removing right now. For your own files, run a <button className="fs-link" onClick={() => useStore.getState().setStartScreen(true)}>disk scan</button>.</>;
  const top = [...free].sort((a, b) => b.i.total - a.i.total)[0];
  const busy = [...rows.filter((r) => r.i.blocked)].sort((a, b) => b.i.total - a.i.total)[0];
  const look = free.filter((r) => !r.i.preselect).length;
  return (
    <>
      There’s <b className="tnum">{formatBytes(found)}</b> you can get back.
      {top && <> The biggest is <b>{top.i.label}</b> ({formatBytes(top.i.total)}).</>}
      {busy && <> <b>{busy.i.label}</b> ({formatBytes(busy.i.total)}) is in use{busy.i.holder ? <> by {busy.i.holder}</> : null}, so I left it out.</>}
      {look > 0 && <> {look} {look === 1 ? 'thing needs' : 'things need'} a look before {look === 1 ? 'it goes' : 'they go'}, so they’re not picked.</>}
    </>
  );
}

function Checklist({ step, startedAt, rows }: { step: Step | null; startedAt: number; rows: Row[] }) {
  const measuring = step && step.label.startsWith('Measuring');
  const [, tick] = useState(0);
  useEffect(() => { const id = window.setInterval(() => tick((n) => n + 1), 500); return () => window.clearInterval(id); }, []);
  const items: { text: string; state: 'done' | 'now' | 'todo' }[] = [
    { text: 'What’s open right now', state: 'done' },
    { text: 'Projects, caches, Xcode, Docker, Trash', state: measuring ? 'done' : 'now' },
    { text: measuring ? step!.label : 'Measuring every folder', state: measuring ? 'now' : 'todo' },
    { text: 'Sorting out what’s safe', state: 'todo' },
  ];
  const secs = Math.max(0, Math.round((Date.now() - startedAt) / 1000));
  return (
    <div className="fs-check-list">
      <h3>Checking</h3>
      {items.map((it) => (
        <div key={it.text} className="fs-step" data-state={it.state}>
          <span className="fs-step-dot">{it.state === 'done' && <Check className="w-2.5 h-2.5" strokeWidth={3.2} />}</span>
          <span className="tnum">{it.text}</span>
        </div>
      ))}
      <p className="fs-fine tnum">{rows.length} found · {secs}s</p>
    </div>
  );
}

function PlanPanel({ t }: { t: ReturnType<typeof tally> }) {
  const vol = useStore((s) => s.overview?.volume ?? s.info?.volume);
  const selected = useFind((s) => s.selected);
  const big = useCountUp(t.now + t.tool, 0.45);
  const holders = [...new Set(t.waitingRows.map((r) => r.i.holder).filter(Boolean))] as string[];
  const go = () => {
    // Only stop to confirm when something needs a decision.
    if (t.waiting || t.tool) useFind.getState().set({ confirming: true });
    else clean();
  };
  return (
    <div className="fs-plan-panel">
      <h3>Your plan</h3>
      <div className="fs-big tnum">{t.tool > 0 && <small className="fs-upto">up to </small>}{formatBytes(big)}</div>
      <p className="fs-sub">{t.count ? `${t.count} ${t.count === 1 ? 'thing' : 'things'} picked` : 'Nothing picked yet'}</p>
      <div className="fs-split">
        <div className="fs-split-row"><i style={{ background: 'var(--fs-accent)' }} /><span>Frees right away</span><b className="tnum">{formatBytes(t.now)}</b></div>
        {t.tool > 0 && <div className="fs-split-row"><i style={{ background: '#a78bfa' }} /><span>Up to, the tool keeps what’s still in use</span><b className="tnum">{formatBytes(t.tool)}</b></div>}
        {t.waiting > 0 && (
          <div className="fs-split-row"><i style={{ background: 'var(--fs-accent-soft)' }} />
            <span>Waiting on you<br /><span className="fs-dim">{holders.length ? `Quit ${holders.join(' and ')}, then check again` : 'In use right now'}</span><br />
              <button className="fs-link" onClick={() => find(useFind.getState().intent, useFind.getState().asked, selected)}>Check again →</button>
            </span>
            <b className="tnum">{formatBytes(t.waiting)}</b>
          </div>
        )}
        {vol?.total ? <div className="fs-split-row fs-dim"><i style={{ background: 'var(--fs-line-strong)' }} /><span>{formatBytes(vol.available + t.now)} free afterwards</span></div> : null}
      </div>
      <button className="fs-go" disabled={!t.count || (t.now + t.tool === 0)} onClick={go}>
        {t.tool > 0 ? `Clean up to ${formatBytes(t.now + t.tool)}` : `Clean ${formatBytes(t.now)}`}
      </button>
      <p className="fs-fine">Build output and caches are deleted. They rebuild or download again by themselves. Nothing you made is touched.</p>
    </div>
  );
}

function Confirm({ t, rows }: { t: ReturnType<typeof tally>; rows: Row[] }) {
  const set = useFind((s) => s.set);
  const selected = useFind((s) => s.selected);
  const tools = rows.filter((r) => selected.has(r.i.id) && r.g.method === 'command' && !r.i.blocked);
  return (
    <motion.div className="fs-sheet" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={() => set({ confirming: false })}>
      <motion.div className="fs-sheet-card" initial={{ opacity: 0, y: 14, scale: 0.97 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: 8, scale: 0.98 }} transition={{ type: 'spring', stiffness: 420, damping: 30 }} onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Confirm cleanup">
        <div className="fs-sheet-head"><GlassFolder mood="wobble" /><h2>Clean {t.tool ? 'up to ' : ''}{formatBytes(t.now + t.tool)}?</h2></div>
        <p className="fs-sheet-lead">Everything here rebuilds or downloads again by itself.</p>
        {t.now > 0 && <div className="fs-sheet-line"><PillTag pill={{ text: 'Deleted', tone: 'safe' }} /><span>Build leftovers, caches and Trash</span><b className="tnum">{formatBytes(t.now)}</b></div>}
        {tools.map((r) => <div key={r.i.id} className="fs-sheet-line"><PillTag pill={{ text: 'Tool decides', tone: 'tool' }} /><span>{r.g.title}: {r.g.after}</span><b className="tnum">up to {formatBytes(r.i.total)}</b></div>)}
        {t.waitingRows.map((r) => <div key={r.i.id} className="fs-sheet-line"><PillTag pill={{ text: 'Skipped', tone: 'busy' }} /><span>{r.i.label}: {r.i.blocked}</span><b className="tnum fs-dim">{formatBytes(r.i.total)}</b></div>)}
        <div className="fs-sheet-actions">
          <button className="fs-btn" onClick={() => set({ confirming: false })}>Back</button>
          <button className="fs-btn fs-btn-primary" onClick={clean} autoFocus>Clean {t.tool ? 'up to ' : ''}{formatBytes(t.now + t.tool)}</button>
        </div>
      </motion.div>
    </motion.div>
  );
}

function Cleaning() {
  const progress = useFind((s) => s.progress);
  const report = useFind((s) => s.report);
  const selected = useFind((s) => s.selected);
  const t = tally(rowsOf(report), selected);
  const total = t.now + t.tool;
  const freed = useCountUp(Math.min(total, progress?.freed_bytes ?? 0), 0.6);
  const frac = progress?.count ? progress.index / progress.count : 0;
  // A card flies into the folder for every item as it's handled.
  const [cards, setCards] = useState<{ id: number; label: string; side: number }[]>([]);
  const n = useRef(0);
  useEffect(() => {
    if (!progress?.label || progress.phase !== 'clean') return;
    const id = ++n.current;
    setCards((c) => [...c.slice(-5), { id, label: progress.label, side: id % 2 ? -1 : 1 }]);
  }, [progress?.label, progress?.phase]);
  return (
    <motion.section {...fade} className="fs-center">
      <div className="fs-fly-zone">
        <AnimatePresence>
          {cards.map((c) => (
            <motion.div key={c.id} className="fs-fly"
              initial={{ opacity: 0, x: c.side * 260, y: -120, rotate: c.side * 14, scale: 1 }}
              animate={{ opacity: [0, 1, 1, 0], x: 0, y: 30, rotate: 0, scale: 0.35 }}
              transition={{ duration: 0.9, ease: [0.5, 0, 0.75, 0] }}
              onAnimationComplete={() => setCards((cs) => cs.filter((x) => x.id !== c.id))}
            >{c.label}</motion.div>
          ))}
        </AnimatePresence>
        <GlassFolder big mood="eat" />
      </div>
      <h1 className="fs-count tnum">{formatBytes(freed)}</h1>
      <p className="fs-now">{progress?.label ?? 'Starting…'}</p>
      <div className="fs-bar"><motion.span animate={{ width: `${Math.max(3, frac * 100)}%` }} transition={{ type: 'spring', stiffness: 80, damping: 20 }} /></div>
    </motion.section>
  );
}

function Confetti() {
  const pieces = useMemo(() => Array.from({ length: 70 }, (_, i) => ({ left: Math.random() * 100, delay: Math.random() * 0.7, dur: 2 + Math.random() * 1.4, color: ['#5b8cff', '#2fbf86', '#f5a524', '#ff6b9a', '#a78bfa'][i % 5] })), []);
  return <div className="fs-confetti" aria-hidden="true">{pieces.map((p, i) => <i key={i} style={{ left: `${p.left}%`, background: p.color, animationDelay: `${p.delay}s`, animationDuration: `${p.dur}s` }} />)}</div>;
}

function Done() {
  const outcome = useFind((s) => s.outcome)!;
  const report = useFind((s) => s.report);
  const selected = useFind((s) => s.selected);
  const freedBytes = outcome.result.freed_bytes;
  const freed = useCountUp(freedBytes, 1.2);
  const gained = Math.max(0, outcome.after.available - outcome.before.available);
  const rows = rowsOf(report);
  const busy = outcome.result.failures.filter((f) => rows.some((r) => r.i.label === f.label && r.i.blocked));
  const other = outcome.result.failures.filter((f) => !busy.includes(f));
  const holders = [...new Set(rows.filter((r) => selected.has(r.i.id) && r.i.blocked).map((r) => r.i.holder).filter(Boolean))] as string[];
  const waitingBytes = rows.filter((r) => selected.has(r.i.id) && r.i.blocked).reduce((n, r) => n + r.i.total, 0);
  const lookBytes = rows.filter((r) => !selected.has(r.i.id) && !r.i.blocked).reduce((n, r) => n + r.i.total, 0);
  const [showOther, setShowOther] = useState(false);
  return (
    <motion.section {...fade} className="fs-center">
      {freedBytes > 0 && <Confetti />}
      <GlassFolder big mood="sleep" />
      <p className="fs-kicker">{gained > 0 ? 'Freed up, confirmed by macOS' : freedBytes > 0 ? 'Freed up' : 'Nothing was removed'}</p>
      <h1 className="fs-count fs-count-big tnum">{formatBytes(freed)}</h1>
      {gained === 0 && freedBytes > 0 && <p className="fs-fine">macOS can take a minute to report the new free space.</p>}
      <div className="fs-results">
        {busy.length > 0 && (
          <div className="fs-result">
            <GlassFolder color="#9db4ff" />
            <span>{busy.length === 1 ? `${busy[0].label} is still waiting` : `${busy.length} things are still waiting`}<br /><span className="fs-dim">{holders.length ? `In use by ${holders.join(' and ')}` : 'In use right now'}</span></span>
            <button className="fs-link" onClick={() => find(useFind.getState().intent, '', selected)}>{holders.length ? `Quit ${holders[0]}, then check again` : 'Check again'} · {formatBytes(waitingBytes)}</button>
          </div>
        )}
        {lookBytes > MIN_ITEM * 4 && (
          <div className="fs-result">
            <GlassFolder color="#f5a524" />
            <span>{formatBytes(lookBytes)} you left for later<br /><span className="fs-dim">Things that needed a look, or you unticked</span></span>
            <button className="fs-link" onClick={() => find('safe', '')}>Go through it →</button>
          </div>
        )}
        <div className="fs-result">
          <GlassFolder color="#2fbf86" />
          <span>Your own big files<br /><span className="fs-dim">Old projects, videos, downloads and duplicates</span></span>
          <button className="fs-link" onClick={() => useStore.getState().setPage('suggestions')}>Look for more <ArrowRight className="inline w-3 h-3" /></button>
        </div>
        {other.length > 0 && (
          <div className="fs-result fs-result-col">
            <button className="fs-link" onClick={() => setShowOther(!showOther)}>{other.length} {other.length === 1 ? 'item' : 'items'} couldn’t be removed</button>
            {showOther && <ul className="fs-fails selectable">{other.map((f) => <li key={f.label + f.error}><b>{f.label}</b>: {f.error}</li>)}</ul>}
          </div>
        )}
      </div>
      <div className="fs-actions">
        <button className="fs-btn" onClick={() => useStore.getState().setPage('quarantine')}>Open quarantine</button>
        <button className="fs-btn fs-btn-primary" onClick={() => useFind.getState().set({ phase: 'start', asked: '', report: null })}>Done</button>
      </div>
    </motion.section>
  );
}
