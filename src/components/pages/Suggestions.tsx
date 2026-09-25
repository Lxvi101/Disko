import React, { useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Sparkles, Plus, Check, Eye, ChevronDown, ChevronLeft, ChevronRight, Copy, Copy as Dup, FolderOpen, Ghost, RefreshCw, Square, Terminal } from 'lucide-react';
import { useStore } from '../../store';
import { api, errorText, onCodexEvent } from '../../lib/api';
import { formatBytes, shortPath, formatDate, uid } from '../../lib/format';
import { Empty, SegmentedControl, Spinner } from '../ui';
import { extractSuggestions } from '../Assistant';
import { age } from './Junk';
import type { DupGroup, RemnantGroup, RemnantMember, StagedItem, Suggestion } from '../../types';
import { openItemMenu } from '../ContextMenu';

type Tab = 'ai' | 'duplicates' | 'leftovers';

const MEMBER_KIND: Record<string, string> = {
  cache: 'Cache',
  http: 'HTTP cache',
  'saved-state': 'Window state',
  logs: 'Logs',
  webkit: 'Web storage',
  state: 'Application Support',
  container: 'Container',
  group: 'Group container',
  prefs: 'Preferences',
  scripts: 'Scripts',
  cookies: 'Cookies',
};

export function memberToStaged(g: RemnantGroup, m: RemnantMember): StagedItem {
  const kind = m.kind === 'prefs' || m.kind === 'cookies' ? 'file' : 'directory';
  const persistent = m.persistent ? ' May hold settings or documents of that app.' : '';
  return {
    path: m.path,
    name: m.path.split('/').filter(Boolean).pop() ?? m.path,
    kind,
    total: m.total,
    category: m.persistent ? 'review' : 'rebuildable',
    reason: `${MEMBER_KIND[m.kind] ?? m.kind} left behind by ${g.id}, which is no longer installed.${persistent}`,
    tag: 'orphaned-app',
    action: 'quarantine',
    source: 'remnant',
  };
}

function openClean(items: StagedItem[]) {
  if (!items.length || useStore.getState().cleanupBusy) return;
  useStore.setState({ cleanupSelection: items, cleanupMode: 'quarantine', cleanModalOpen: true });
}

const nameOf = (p: string) => p.split('/').filter(Boolean).pop() ?? p;

// Findings survive tab switches; they are tied to the scan they were made from.
let aiCache: { db: string; at: number; text: string; suggestions: Suggestion[] } | null = (() => {
  try { return JSON.parse(localStorage.getItem('disko.aiPicks') ?? 'null'); } catch { return null; }
})();
let dupCache: { key: string; groups: DupGroup[] } | null = null;

export const SuggestionsPage: React.FC = () => {
  const remnants = useStore((s) => s.remnants);
  const [tab, setTab] = useState<Tab>('ai');
  const leftoverBytes = (remnants?.groups ?? []).reduce((a, g) => a + g.total, 0);
  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[1000px] mx-auto px-8 pt-7 pb-8">
        <header className="flex items-end justify-between gap-4 mb-6">
          <div>
            <h1 className="text-[26px] font-bold tracking-tight leading-none">Suggestions</h1>
            <p className="text-[13px] mt-2" style={{ color: 'var(--muted)' }}>Things worth a second look that aren’t obvious junk.</p>
          </div>
          <SegmentedControl value={tab} onChange={setTab} options={[
            { value: 'ai', label: <><Sparkles className="w-3.5 h-3.5" /> AI picks</> },
            { value: 'duplicates', label: <><Dup className="w-3.5 h-3.5" /> Duplicates</> },
            { value: 'leftovers', label: <><Ghost className="w-3.5 h-3.5" /> Leftovers{leftoverBytes ? <span className="tnum opacity-60">{formatBytes(leftoverBytes)}</span> : null}</> },
          ]} />
        </header>
        {tab === 'ai' ? <AiPicks /> : tab === 'duplicates' ? <Duplicates /> : <Leftovers />}
      </div>
    </div>
  );
};

const AI_PROMPT = `Find the best cleanup opportunities on this Mac that go beyond generic junk. Disko already lists node_modules, virtualenvs, build output, caches, logs and installers on its own Junk page, so do not repeat those unless one is exceptionally large or clearly abandoned.
Look for: projects and folders untouched for a long time, large videos, disk images and archives that look forgotten, old device backups, duplicate app versions, unused language runtimes and toolchains, Docker images and volumes, virtual machines, downloaded models, and anything else large that looks forgotten.
Query the scan database and check modification dates. Be specific and conservative: never suggest documents, photos or source code that may be irreplaceable without saying so and setting confidence to low.
Reply with two or three sentences summarising what you found, then a disko-actions block with at most 15 suggestions, largest first. Use action "command" with the exact command when a tool manages the data.`;

function AiPicks() {
  const info = useStore((s) => s.info);
  const busy = useStore((s) => s.cleanupBusy);
  const toast = useStore((s) => s.toast);
  const db = info?.db_path ?? '';
  const [result, setResult] = useState(aiCache?.db === db ? aiCache : null);
  const [running, setRunning] = useState<string | null>(null);
  const [status, setStatus] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const req = useRef<string | null>(null);

  useEffect(() => {
    let un: (() => void) | null = null;
    onCodexEvent((p) => {
      if (p.request_id !== req.current) return;
      const item = p.event?.item ?? {};
      if (item.type === 'command_execution') setStatus(String(item.command ?? '').replace(/^\/bin\/z?sh -lc\s+/, '').replace(/^'(.*)'$/s, '$1').slice(0, 140));
      else if (item.type === 'reasoning') setStatus('Thinking…');
    }).then((u) => (un = u));
    return () => { un?.(); };
  }, []);

  const run = async () => {
    const st = useStore.getState();
    const requestId = uid();
    req.current = requestId;
    setRunning(requestId); setError(null); setStatus('Reading the scan…');
    try {
      const res = await api.chat({ requestId, prompt: AI_PROMPT, model: st.model, effort: st.effort, context: { page: 'suggestions', current_path: st.rootPath, items: [], staged: [], history: [] } });
      if (!res.success && !res.text) throw new Error(res.error ?? 'Codex failed');
      const { clean, suggestions } = extractSuggestions(res.text ?? '');
      const next = { db, at: Date.now(), text: clean, suggestions };
      aiCache = next;
      try { localStorage.setItem('disko.aiPicks', JSON.stringify(next)); } catch {}
      setResult(next);
      setSelected(new Set());
    } catch (e) {
      setError(errorText(e));
    } finally {
      req.current = null;
      setRunning(null);
    }
  };
  const stop = () => { if (running) api.cancelChat(running).catch(() => {}); };

  if (!info?.codex_version) {
    return <Empty icon={<Sparkles className="w-6 h-6" />} title="AI picks need the Codex CLI" body="Install Codex from chatgpt.com/codex, sign in, then reopen Disko. It reads your scan and suggests what to remove; nothing is removed without you." />;
  }
  if (running) {
    return (
      <div className="ai-card flex flex-col items-center text-center py-14">
        <Spinner size={20} />
        <h2 className="text-[15px] font-semibold mt-4">Looking through your scan…</h2>
        <p className="mono text-[11px] mt-2 max-w-[560px] truncate" style={{ color: 'var(--dim)' }}>{status}</p>
        <button className="btn btn-ghost mt-5" onClick={stop}><Square className="w-3 h-3" /> Stop</button>
      </div>
    );
  }
  if (!result) {
    return (
      <div className="ai-card flex flex-col items-center text-center py-14 px-8">
        <span className="ai-orb"><Sparkles className="w-5 h-5" /></span>
        <h2 className="text-[17px] font-semibold mt-4">Let AI find what you forgot about</h2>
        <p className="text-[12.5px] mt-1.5 max-w-[440px] leading-relaxed" style={{ color: 'var(--muted)' }}>Old projects, forgotten videos, unused toolchains, VM images and backups. Codex reads your scan read-only and explains each pick. You decide what goes.</p>
        {error && <p role="alert" className="text-[12px] mt-3 selectable" style={{ color: 'var(--danger)' }}>{error}</p>}
        <button className="btn btn-primary mt-5 !px-4" onClick={run}><Sparkles className="w-3.5 h-3.5" /> Find suggestions</button>
        <p className="text-[11px] mt-3" style={{ color: 'var(--dim)' }}>Takes a minute or two · uses your Codex plan</p>
      </div>
    );
  }

  const picks = result.suggestions;
  const chosen = picks.filter((s) => selected.has(s.path) && s.action !== 'command');
  return (
    <div>
      <div className="flex items-start gap-3">
        <span className="ai-orb small"><Sparkles className="w-3.5 h-3.5" /></span>
        <div className="md flex-1 min-w-0 text-[13px]"><ReactMarkdown remarkPlugins={[remarkGfm]}>{result.text || 'Here is what stood out.'}</ReactMarkdown></div>
        <button className="btn btn-ghost btn-sm flex-shrink-0" onClick={run} title={`Found ${new Date(result.at).toLocaleString()}`}><RefreshCw className="w-3 h-3" /> Ask again</button>
      </div>
      {error && <p role="alert" className="text-[12px] mt-3 selectable" style={{ color: 'var(--danger)' }}>{error}</p>}
      <div className="junk-list mt-5">
        {picks.map((s) => {
          const command = s.action === 'command';
          return (
            <label key={s.path} className="junk-row group !items-start !py-3" aria-disabled={command}>
              <input type="checkbox" className="mt-1" disabled={command || busy} checked={selected.has(s.path)} onChange={() => setSelected((old) => { const n = new Set(old); n.has(s.path) ? n.delete(s.path) : n.add(s.path); return n; })} />
              <span className="flex-1 min-w-0">
                <span className="flex items-center gap-2 min-w-0">
                  <span className="text-[13px] font-medium truncate">{nameOf(s.path)}</span>
                  <span className={`junk-kind ${s.confidence === 'low' ? 'warn' : ''}`}>{s.confidence === 'high' ? 'Confident' : s.confidence === 'low' ? 'Check first' : 'Likely'}</span>
                </span>
                <span className="block text-[11.5px] truncate mt-0.5 selectable" style={{ color: 'var(--dim)' }} title={s.path}>{shortPath(s.path, info?.home ?? '')}</span>
                <span className="block text-[12px] leading-relaxed mt-1.5" style={{ color: 'var(--muted)' }}>{s.reason}</span>
                {command && s.command && (
                  <button className="junk-command mt-2" onClick={(e) => { e.preventDefault(); navigator.clipboard?.writeText(s.command!).then(() => toast({ kind: 'success', title: 'Command copied', detail: s.command! })).catch(() => {}); }}>
                    <Terminal className="w-3 h-3 flex-shrink-0" /><span className="truncate">{s.command}</span><Copy className="w-3 h-3 flex-shrink-0 opacity-60" />
                  </button>
                )}
              </span>
              <span className="tnum text-[13px] font-semibold flex-shrink-0">{s.bytes ? formatBytes(s.bytes) : ''}</span>
              <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Show in Finder" onClick={(e) => { e.preventDefault(); api.reveal(s.path).catch(() => {}); }}><FolderOpen className="w-3.5 h-3.5" /></button>
            </label>
          );
        })}
        {!picks.length && <p className="py-10 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>No specific picks this time.</p>}
      </div>
      {chosen.length > 0 && (
        <div className="action-bar">
          <span className="text-[12.5px] flex-1" style={{ color: 'var(--muted)' }}><strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(chosen.reduce((n, s) => n + (s.bytes ?? 0), 0))}</strong> selected from {chosen.length}</span>
          <button className="btn btn-ghost" disabled={busy} onClick={() => setSelected(new Set())}>Clear</button>
          <button className="btn btn-primary" disabled={busy} onClick={() => openClean(chosen.map((s) => ({ path: s.path, name: nameOf(s.path), kind: 'directory', total: s.bytes ?? 0, category: 'review', reason: s.reason, source: 'assistant' })))}>Clean up…</button>
        </div>
      )}
    </div>
  );
}

const DUP_PAGE = 20;

function Duplicates() {
  const home = useStore((s) => s.info?.home ?? '');
  const key = `${useStore((s) => s.info?.db_path)}:${useStore((s) => s.dataVersion)}`;
  const busy = useStore((s) => s.cleanupBusy);
  const [groups, setGroups] = useState<DupGroup[] | null>(dupCache?.key === key ? dupCache.groups : null);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [page, setPage] = useState(0);
  // Every copy except the oldest starts selected; the oldest is most likely the original.
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (dupCache?.key === key && !refresh) { setSelected(new Set(dupCache.groups.flatMap((g) => g.files.slice(1).map((f) => f.path)))); return; }
    let live = true;
    setGroups(null); setError(null);
    api.duplicates().then((g) => {
      if (!live) return;
      dupCache = { key, groups: g };
      setGroups(g);
      setSelected(new Set(g.flatMap((x) => x.files.slice(1).map((f) => f.path))));
    }).catch((e) => live && setError(errorText(e)));
    return () => { live = false; };
  }, [key, refresh]);

  if (error) return <Empty icon={<Dup className="w-6 h-6" />} title="Could not compare files" body={error} action={<button className="btn" onClick={() => setRefresh((n) => n + 1)}>Try again</button>} />;
  if (!groups) return <div className="py-20 flex items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--muted)' }}><Spinner /> Comparing files byte by byte…</div>;
  if (!groups.length) return <Empty icon={<Dup className="w-6 h-6" />} title="No duplicates found" body="No identical photos, videos, documents, archives or installers over 1 MB." />;

  const wasted = groups.reduce((n, g) => n + g.allocated * (g.files.length - 1), 0);
  const pages = Math.ceil(groups.length / DUP_PAGE);
  const current = Math.min(page, pages - 1);
  const shown = groups.slice(current * DUP_PAGE, current * DUP_PAGE + DUP_PAGE);
  const chosen = groups.flatMap((g) => g.files.filter((f) => selected.has(f.path)).map((f) => ({ g, f })));
  const chosenBytes = chosen.reduce((n, c) => n + c.g.allocated, 0);
  const toggle = (g: DupGroup, path: string) => setSelected((old) => {
    const next = new Set(old);
    if (next.has(path)) next.delete(path);
    // Never let every copy of a file be selected.
    else if (g.files.filter((f) => next.has(f.path)).length < g.files.length - 1) next.add(path);
    return next;
  });

  return (
    <div>
      <p className="text-[13px]" style={{ color: 'var(--muted)' }}>
        <strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(wasted)}</strong> in {groups.length} sets of identical files. The oldest copy of each is kept.
        <button className="btn btn-ghost btn-sm ml-2" onClick={() => setRefresh((n) => n + 1)}><RefreshCw className="w-3 h-3" /> Compare again</button>
      </p>
      <div className="mt-4 space-y-3">
        {shown.map((g) => (
          <section key={g.files[0].path} className="junk-list">
            <div className="flex items-center gap-3 px-4 py-2.5" style={{ borderBottom: '1px solid var(--line)' }}>
              <span className="text-[13px] font-semibold truncate flex-1 min-w-0">{g.files[0].name}</span>
              <span className="tnum text-[12px]" style={{ color: 'var(--muted)' }}>{g.files.length} copies · {formatBytes(g.size)} each</span>
            </div>
            {g.files.map((f) => {
              const on = selected.has(f.path);
              return (
                <label key={f.path} className="junk-row group">
                  <input type="checkbox" disabled={busy} checked={on} onChange={() => toggle(g, f.path)} />
                  <span className="flex-1 min-w-0">
                    <span className="block text-[12.5px] truncate selectable" title={f.path}>{shortPath(f.path.slice(0, f.path.length - f.name.length - 1), home)}</span>
                  </span>
                  {!on && <span className="junk-kind">Keep</span>}
                  <span className="text-[11.5px] w-[76px] text-right flex-shrink-0" style={{ color: 'var(--dim)' }} title={f.mtime ? `Modified ${formatDate(f.mtime)}` : undefined}>{age(f.mtime)}</span>
                  <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Show in Finder" onClick={(e) => { e.preventDefault(); api.reveal(f.path).catch(() => {}); }}><FolderOpen className="w-3.5 h-3.5" /></button>
                </label>
              );
            })}
          </section>
        ))}
      </div>
      {pages > 1 && (
        <nav className="flex items-center justify-between mt-4 text-[12px]" style={{ color: 'var(--muted)' }}>
          <span className="tnum">Sets {current * DUP_PAGE + 1}–{Math.min(groups.length, (current + 1) * DUP_PAGE)} of {groups.length}</span>
          <span className="flex items-center gap-1">
            <button className="btn btn-ghost btn-icon" disabled={current === 0} onClick={() => setPage(current - 1)} aria-label="Previous page"><ChevronLeft className="w-4 h-4" /></button>
            <span className="tnum px-2">{current + 1} / {pages}</span>
            <button className="btn btn-ghost btn-icon" disabled={current >= pages - 1} onClick={() => setPage(current + 1)} aria-label="Next page"><ChevronRight className="w-4 h-4" /></button>
          </span>
        </nav>
      )}
      <p className="text-[11px] mt-4" style={{ color: 'var(--dim)' }}>Copies made with Finder’s Duplicate can share storage on APFS, so removing them may free less than shown.</p>
      {chosen.length > 0 && (
        <div className="action-bar">
          <span className="text-[12.5px] flex-1" style={{ color: 'var(--muted)' }}><strong className="tnum" style={{ color: 'var(--text)' }}>{formatBytes(chosenBytes)}</strong> in {chosen.length} extra {chosen.length === 1 ? 'copy' : 'copies'}</span>
          <button className="btn btn-ghost" disabled={busy} onClick={() => setSelected(new Set())}>Clear</button>
          <button className="btn btn-primary" disabled={busy} onClick={() => openClean(chosen.map(({ g, f }) => ({ path: f.path, name: f.name, kind: 'file', total: g.allocated, category: 'review', reason: `Identical copy of a file kept elsewhere (${g.files.length} copies).`, tag: 'user-files', action: 'quarantine', source: 'suggestion' })))}>Remove copies…</button>
        </div>
      )}
    </div>
  );
}

function Leftovers() {
  const { remnants, isStaged, toggleStage, stageMany, info } = useStore();
  const [rebuilding, setRebuilding] = useState(false);
  const groups = remnants?.groups ?? [];
  const preselect = groups.flatMap((g) => g.members.filter((m) => m.preselect).map((m) => memberToStaged(g, m)));
  const allStaged = preselect.length > 0 && preselect.every((m) => isStaged(m.path));
  const rebuild = async () => {
    setRebuilding(true);
    try { useStore.setState({ remnants: await api.remnants(true) }); } catch {}
    setRebuilding(false);
  };
  return (
    <div>
      <div className="flex items-center gap-3 mb-4">
        <p className="text-[13px] flex-1" style={{ color: 'var(--muted)' }}>Data left behind by apps that are no longer installed. Caches are safe; settings and containers are never preselected.</p>
        <button className="btn btn-ghost btn-icon" title="Look again" onClick={rebuild} disabled={rebuilding}>{rebuilding ? <Spinner /> : <RefreshCw className="w-3.5 h-3.5" />}</button>
        <button className="btn" onClick={() => stageMany(preselect)} disabled={!preselect.length || allStaged}>
          {allStaged ? <><Check className="w-3.5 h-3.5" /> Caches collected</> : <><Plus className="w-3.5 h-3.5" /> Collect caches · {formatBytes(preselect.reduce((a, m) => a + m.total, 0))}</>}
        </button>
      </div>
      <LeftoversList groups={groups} agents={remnants?.agents ?? []} home={info?.home ?? ''} isStaged={isStaged} onToggle={(g, m) => toggleStage(memberToStaged(g, m))} onToggleAgent={(a) => toggleStage({ path: a.path, name: a.path.split('/').pop() ?? a.path, kind: 'file', total: 0, category: 'review', reason: a.reason, tag: 'orphaned-service', action: 'quarantine', source: 'remnant' })} loaded={!!remnants} />
    </div>
  );
}

const LeftoversList: React.FC<{
  groups: RemnantGroup[];
  agents: { path: string; label: string; program: string; reason: string }[];
  home: string;
  loaded: boolean;
  isStaged: (p: string) => boolean;
  onToggle: (g: RemnantGroup, m: RemnantMember) => void;
  onToggleAgent: (a: { path: string; label: string; program: string; reason: string }) => void;
}> = ({ groups, agents, home, loaded, isStaged, onToggle, onToggleAgent }) => {
  const [open, setOpen] = useState<Record<string, boolean>>({});
  if (!loaded) {
    return (
      <div className="py-16 flex items-center justify-center gap-2 text-[12.5px]" style={{ color: 'var(--dim)' }}>
        <Spinner /> Comparing installed apps with Library…
      </div>
    );
  }
  if (groups.length === 0 && agents.length === 0) {
    return (
      <div className="py-16 text-center text-[12.5px]" style={{ color: 'var(--dim)' }}>
        No leftovers found. Every bundle identifier in Library belongs to an installed app.
      </div>
    );
  }
  return (
    <div className="grid grid-cols-1 gap-2">
      {groups.map((g) => {
        const likely = g.confidence === 'likely';
        const expanded = open[g.id] ?? false;
        const stagedCount = g.members.filter((m) => isStaged(m.path)).length;
        return (
          <div key={g.id} className="card p-3.5">
            <div className="flex items-start gap-3">
              <span className="w-9 h-9 rounded-xl flex items-center justify-center flex-shrink-0 mt-0.5" style={{ background: 'rgba(167, 139, 250, 0.14)', color: '#c4b5fd' }}>
                <Ghost className="w-4 h-4" />
              </span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-[13px] font-semibold truncate min-w-0">{g.name}</span>
                  <span className="chip" style={{ fontSize: 10, color: likely ? '#c4b5fd' : 'var(--warn)' }}>{likely ? 'Likely uninstalled' : 'Needs confirmation'}</span>
                  {stagedCount > 0 && <span className="chip" style={{ fontSize: 10 }}>{stagedCount} collected</span>}
                </div>
                <div className="mono text-[10.5px] truncate mt-0.5 selectable" style={{ color: 'var(--dim)' }}>{g.id}</div>
                <ul className="text-[11.5px] leading-snug mt-1.5 space-y-0.5" style={{ color: 'var(--muted)' }}>
                  {g.evidence.slice(0, expanded ? undefined : 2).map((e) => (
                    <li key={e}>· {e}</li>
                  ))}
                </ul>
                {g.newest_mtime ? <div className="text-[11px] mt-1" style={{ color: 'var(--dim)' }}>Last changed {formatDate(g.newest_mtime)}</div> : null}
              </div>
              <div className="flex flex-col items-end gap-2 flex-shrink-0">
                <span className="mono text-[15px] font-semibold">{formatBytes(g.total)}</span>
                <button className="btn btn-ghost btn-sm" onClick={() => setOpen({ ...open, [g.id]: !expanded })}>
                  {g.members.length} item{g.members.length === 1 ? '' : 's'} <ChevronDown className="w-3 h-3 transition-transform" style={{ transform: expanded ? 'rotate(180deg)' : undefined }} />
                </button>
              </div>
            </div>
            <AnimatePresence initial={false}>
              {expanded && (
                <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: 0.16 }} className="overflow-hidden">
                  <div className="mt-3 pt-2" style={{ borderTop: '1px solid var(--line)' }}>
                    {g.members.map((m) => {
                      const s = isStaged(m.path);
                      return (
                        <div key={m.path} className="group flex items-center gap-2.5 px-1 h-8 rounded-lg row-hover" onContextMenu={(e) => openItemMenu(e, memberToStaged(g, m))}>
                          <span className="chip flex-shrink-0" style={{ fontSize: 10, color: m.persistent ? 'var(--warn)' : '#6ee7b7' }}>{MEMBER_KIND[m.kind] ?? m.kind}</span>
                          <span className="mono text-[10.5px] truncate flex-1 min-w-0 selectable" title={m.path}>{shortPath(m.path, home)}</span>
                          <span className="tnum text-[11.5px] flex-shrink-0" style={{ color: 'var(--muted)' }}>{m.total ? formatBytes(m.total) : '—'}</span>
                          <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100" title="Reveal in Finder" onClick={() => api.reveal(m.path).catch(() => {})}><Eye className="w-3.5 h-3.5" /></button>
                          <button className="btn btn-ghost btn-icon !p-1" title={m.persistent ? 'May hold settings or documents; collect only if you are sure' : 'Collect'} style={s ? { color: 'var(--text)' } : undefined} onClick={() => onToggle(g, m)}>{s ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}</button>
                        </div>
                      );
                    })}
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        );
      })}
      {agents.length > 0 && (
        <div className="card p-3.5 mt-2">
          <div className="text-[12.5px] font-semibold mb-1">Launch agents whose program is gone</div>
          <p className="text-[11.5px] mb-2" style={{ color: 'var(--muted)' }}>launchd keeps trying to start these. They are tiny, but removing them stops the failures.</p>
          {agents.map((a) => {
            const s = isStaged(a.path);
            return (
              <div key={a.path} className="group flex items-center gap-2.5 px-1 h-8 rounded-lg row-hover" onContextMenu={(e) => openItemMenu(e, { path: a.path, name: a.path.split('/').pop() ?? a.path, kind: 'file', total: 0, category: 'review', reason: a.reason, tag: 'orphaned-service', action: 'quarantine', source: 'remnant' })}>
                <span className="text-[12px] truncate flex-1 min-w-0">{a.label}</span>
                <span className="mono text-[10.5px] truncate flex-1 min-w-0" style={{ color: 'var(--dim)' }} title={a.program}>{shortPath(a.program, home)}</span>
                <button className="btn btn-ghost btn-icon !p-1 opacity-0 group-hover:opacity-100" title="Reveal in Finder" onClick={() => api.reveal(a.path).catch(() => {})}><Eye className="w-3.5 h-3.5" /></button>
                <button className="btn btn-ghost btn-icon !p-1" style={s ? { color: 'var(--text)' } : undefined} onClick={() => onToggleAgent(a)}>{s ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}</button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
