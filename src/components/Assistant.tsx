import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowUp, Square, X, ChevronRight, ChevronDown, Plus, Check, Trash2, Undo2, Sparkles, ScanSearch, PackageOpen, HardDrive, Brain, ShieldCheck } from 'lucide-react';
import { useStore } from '../store';
import { api, errorText, onCodexEvent } from '../lib/api';
import { formatBytes, shortPath, uid } from '../lib/format';
import type { ActivityItem, ChatMessage, Suggestion } from '../types';

const ACTIONS_RE = /```disko-actions\s*([\s\S]*?)```/g;

export function extractSuggestions(text: string): { clean: string; suggestions: Suggestion[] } {
  const suggestions: Suggestion[] = [];
  const clean = text
    .replace(ACTIONS_RE, (_, body) => {
      try {
        const j = JSON.parse(body.trim());
        const arr = Array.isArray(j) ? j : j.suggestions ?? j.items ?? [];
        for (const s of arr) {
          if (s && typeof s.path === 'string' && s.path.startsWith('/')) {
            const command = typeof s.command === 'string' && s.command.trim() ? s.command.trim() : undefined;
            const action = s.action === 'command' || (command && s.action !== 'quarantine') ? 'command' : 'quarantine';
            suggestions.push({ path: s.path, bytes: Number(s.bytes) || undefined, reason: String(s.reason ?? ''), confidence: String(s.confidence ?? 'medium'), action, command });
          }
        }
      } catch {}
      return '';
    })
    .trim();
  return { clean, suggestions };
}

function summarizeCommand(cmd: string): string {
  return cmd.replace(/^\/bin\/z?sh -lc\s+/, '').replace(/^'(.*)'$/s, '$1').replace(/^"(.*)"$/s, '$1').slice(0, 160);
}

/** MCP results arrive as text content holding JSON. */
function parseToolResult(item: any): any {
  const sc = item?.result?.structured_content ?? item?.result?.structuredContent;
  if (sc) return sc;
  const text = item?.result?.content?.find?.((c: any) => c?.type === 'text')?.text;
  try { return text ? JSON.parse(text) : null; } catch { return text ?? null; }
}

const TOOL_LABEL: Record<string, { running: string; done: (r: any, a: any) => string }> = {
  quick_scan: { running: 'Sweeping for junk…', done: (r) => `Ran Quick Clean · ${r?.groups?.length ?? 0} groups found` },
  quarantine: { running: 'Moving things to quarantine…', done: (r) => r?.moved?.length ? `Quarantined ${r.moved.length} ${r.moved.length === 1 ? 'item' : 'items'} · ${r.size}` : 'Nothing was moved' },
  quarantine_list: { running: 'Checking the quarantine…', done: (r) => `Looked at the quarantine · ${r?.batches?.length ?? 0} batches` },
  quarantine_restore: { running: 'Restoring…', done: (r) => `Restored ${r?.restored_items ?? 0} items` },
  disk_status: { running: 'Checking free space…', done: (r) => `Checked disk · ${r?.free ?? '?'} free` },
};

export const Assistant: React.FC = () => {
  const { messages, generating, setAssistantOpen, currentPath, staged, info, clearChat, toast, stageMany, isStaged, toggleStage } = useStore();
  const [input, setInput] = useState('');
  const listRef = useRef<HTMLDivElement>(null);
  const activeReq = useRef<string | null>(null);

  useEffect(() => {
    // Unlisten even if the effect is torn down before `listen` resolves (React remounts in dev).
    let live = true;
    let un: (() => void) | null = null;
    onCodexEvent((p) => {
      if (!live) return;
      const st = useStore.getState();
      if (!st.messages.some((m) => m.id === p.request_id)) return;
      const ev = p.event;
      const type: string = ev.type;
      if (type !== 'item.started' && type !== 'item.completed' && type !== 'item.updated') return;
      const item = ev.item ?? {};
      const itemType: string = item.type;
      const id = String(item.id ?? uid());
      const done = type === 'item.completed';
      st.updateMessage(p.request_id, (m) => {
        const activity = [...(m.activity ?? [])];
        const idx = activity.findIndex((a) => a.id === id);
        let entry: ActivityItem | null = null;
        if (itemType === 'command_execution') {
          entry = { id, type: 'command', title: summarizeCommand(String(item.command ?? '')), detail: String(item.aggregated_output ?? '').slice(0, 4000), status: done ? (item.exit_code === 0 || item.exit_code == null ? 'done' : 'failed') : 'running' };
        } else if (itemType === 'mcp_tool_call') {
          const result = done ? parseToolResult(item) : null;
          const failed = done && (item.status === 'failed' || !!item.error || item.result?.is_error || item.result?.isError);
          const label = TOOL_LABEL[item.tool] ?? { running: `Running ${item.tool}…`, done: () => `Ran ${item.tool}` };
          entry = { id, type: 'tool', tool: String(item.tool), args: item.arguments, result, title: failed ? `${item.tool} failed: ${typeof result === 'string' ? result : item.error?.message ?? ''}` : done ? label.done(result, item.arguments) : label.running, status: failed ? 'failed' : done ? 'done' : 'running' };
        } else if (itemType === 'reasoning') {
          entry = { id, type: 'reasoning', title: String(item.text ?? item.summary ?? 'Thinking').slice(0, 400), status: 'done' };
        } else if (itemType === 'error') {
          entry = { id, type: 'error', title: String(item.message ?? 'Error'), status: 'failed' };
        } else if (itemType === 'agent_message' && done) {
          const text = String(item.text ?? '');
          return { text: m.text ? `${m.text}\n\n${text}` : text };
        }
        if (!entry) return {};
        if (idx >= 0) activity[idx] = entry;
        else activity.push(entry);
        return { activity };
      });
      // The agent moved things: bring every view up to date and tell the user.
      if (done && itemType === 'mcp_tool_call' && (item.tool === 'quarantine' || item.tool === 'quarantine_restore')) {
        const r = parseToolResult(item);
        st.refreshJournals(); st.reloadData(); st.refreshCurrent();
        if (item.tool === 'quarantine' && r?.moved?.length) st.toast({ kind: 'success', title: `Disko quarantined ${r.moved.length} ${r.moved.length === 1 ? 'item' : 'items'}`, detail: `${r.size} · empty the quarantine to free the space` });
      }
    }).then((u) => { if (live) un = u; else u(); });
    return () => { live = false; un?.(); };
  }, []);

  const send = useCallback(async (text: string) => {
    const st = useStore.getState();
    if (!text.trim() || st.generating) return;
    const requestId = uid();
    st.pushMessage({ id: uid(), role: 'user', text: text.trim(), createdAt: Date.now() });
    st.pushMessage({ id: requestId, role: 'assistant', text: '', createdAt: Date.now(), model: st.model, streaming: true, activity: [] });
    st.setGenerating(requestId);
    activeReq.current = requestId;
    setInput('');
    const history = st.messages.filter((m) => !m.streaming && m.text).slice(-10).map((m) => ({ role: m.role, text: m.text }));
    try {
      const res = await api.chat({
        requestId,
        prompt: text.trim(),
        model: st.model,
        effort: st.effort,
        agent: true,
        context: {
          page: st.page,
          current_path: st.currentPath,
          items: st.currentItems.slice(0, 25).map((i) => ({ path: i.path, total: i.total, category: i.category, kind: i.kind, tag: i.tag, action: i.action })),
          staged: st.staged.map((i) => ({ path: i.path, total: i.total, category: i.category, kind: i.kind, tag: i.tag ?? '', action: i.action ?? '' })),
          history,
        },
      });
      const finalText = res.text || useStore.getState().messages.find((m) => m.id === requestId)?.text || '';
      const { clean, suggestions } = extractSuggestions(finalText);
      st.updateMessage(requestId, { text: clean || (res.error ? '' : '(no reply)'), streaming: false, suggestions, error: res.success ? null : res.error ?? 'Codex failed', usage: res.usage ?? null });
    } catch (e) {
      st.updateMessage(requestId, { streaming: false, error: errorText(e) });
    } finally {
      st.setGenerating(null);
      activeReq.current = null;
    }
  }, []);

  useEffect(() => {
    const handler = (e: Event) => {
      const q = (e as CustomEvent<string>).detail;
      if (q) send(q);
    };
    window.addEventListener('disko:ask', handler);
    return () => window.removeEventListener('disko:ask', handler);
  }, [send]);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  }, [messages, generating]);

  const stop = async () => {
    if (activeReq.current) await api.cancelChat(activeReq.current).catch(() => {});
  };

  const starters = useMemo(() => {
    const folder = currentPath ? currentPath.split('/').filter(Boolean).pop() : null;
    const base = [
      { icon: Sparkles, label: 'Free up space for me', sub: 'Scans, then quarantines the safe stuff', q: 'Free up space for me. Run a quick scan, quarantine everything that is clearly safe and idle, and tell me what you moved and what you left for me to decide.' },
      { icon: HardDrive, label: 'Where did my space go?', sub: 'A short tour of the biggest things', q: 'Where did my disk space go? Give me a short tour of the biggest things and which of them are safe to clean.' },
      { icon: ScanSearch, label: 'Clean my developer junk', sub: 'Builds, node_modules, caches, simulators', q: 'Clean up my developer junk: old build folders, node_modules, virtualenvs, package caches and Xcode leftovers I have not touched in a while. Quarantine what is clearly safe.' },
      { icon: PackageOpen, label: 'What’s in quarantine?', sub: 'And is it safe to empty?', q: 'What is in my quarantine right now, and is any of it risky to empty?' },
    ];
    if (folder && currentPath !== info?.home) base.splice(2, 0, { icon: ScanSearch, label: `What’s in ${folder}?`, sub: 'Explain this folder', q: `Explain what is taking up space in ${currentPath} and which parts are safe to remove.` });
    if (staged.length) base.unshift({ icon: ShieldCheck, label: `Review my ${staged.length} picks`, sub: 'Flags anything risky', q: 'Review the items I collected for cleanup. Flag anything risky and say what each one costs to recreate.' });
    return base.slice(0, 4);
  }, [currentPath, staged.length, info?.home]);

  const stageSuggestions = (all: Suggestion[]) => {
    const s = all.filter((x) => x.action !== 'command');
    if (s.length === 0) return;
    stageMany(s.map((x) => ({ path: x.path, name: x.path.split('/').filter(Boolean).pop() ?? x.path, kind: 'directory', total: x.bytes ?? 0, category: 'review', reason: x.reason, source: 'assistant' as const })));
    toast({ kind: 'success', title: `Collected ${s.length} item${s.length === 1 ? '' : 's'}` });
  };

  return (
    <aside className="agent h-full flex flex-col">
      <div className="h-[52px] flex items-center gap-2 px-4 flex-shrink-0 drag-region">
        <span className="text-[13px] font-semibold">Agent</span>
        <span className="flex-1" />
        {messages.length > 0 && (
          <button className="no-drag btn btn-ghost btn-icon" title="New chat" onClick={clearChat} disabled={!!generating}><Trash2 className="w-3.5 h-3.5" /></button>
        )}
        <button className="no-drag btn btn-ghost btn-icon" onClick={() => setAssistantOpen(false)} title="Hide (⌘J)"><X className="w-4 h-4" /></button>
      </div>

      <div ref={listRef} className="flex-1 min-h-0 overflow-y-auto px-4 pb-2">
        {messages.length === 0 && (
          <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} className="pt-6">
            <h2 className="text-[19px] font-semibold">What should I tidy up?</h2>
            <p className="text-[12.5px] leading-relaxed mt-1.5" style={{ color: 'var(--muted)' }}>
              I can scan your Mac, explain what’s using space and move the safe stuff into quarantine myself. I never delete anything: you empty the quarantine when you’re happy.
            </p>
            <div className="grid gap-2 mt-5">
              {starters.map((q, i) => (
                <motion.button key={q.label} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.05 * i }} className="agent-starter" onClick={() => send(q.q)} disabled={!info?.codex_path}>
                  <span className="agent-starter-icon"><q.icon className="w-4 h-4" /></span>
                  <span className="flex-1 min-w-0 text-left">
                    <span className="block text-[13px] font-semibold">{q.label}</span>
                    <span className="block text-[11.5px]" style={{ color: 'var(--dim)' }}>{q.sub}</span>
                  </span>
                  <ChevronRight className="w-4 h-4" style={{ color: 'var(--dim)' }} />
                </motion.button>
              ))}
            </div>
            {!info?.codex_path && <p className="text-[12px] mt-4" style={{ color: 'var(--warn)' }}>The agent runs on the Codex CLI, which isn’t installed. Get it from chatgpt.com/codex.</p>}
          </motion.div>
        )}
        {messages.map((m) => (
          <Message key={m.id} m={m} home={info?.home ?? ''} isStaged={isStaged} onToggle={(s) => toggleStage({ path: s.path, name: s.path.split('/').pop() ?? s.path, kind: 'directory', total: s.bytes ?? 0, category: 'review', reason: s.reason, source: 'assistant' })} onStageAll={stageSuggestions} />
        ))}
      </div>

      <div className="p-3 flex-shrink-0">
        <div className="agent-composer">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(input); } }}
            rows={Math.min(6, Math.max(2, input.split('\n').length))}
            placeholder={info?.codex_path ? 'Ask or tell Disko what to clean…' : 'Codex CLI not found'}
            disabled={!info?.codex_path}
            className="w-full bg-transparent outline-none resize-none text-[13px] selectable"
            style={{ color: 'var(--text)' }}
          />
          <div className="flex items-center gap-2 mt-1">
            <ModelPicker />
            <span className="flex-1" />
            {generating ? (
              <button className="agent-send" onClick={stop} title="Stop"><Square className="w-3 h-3 fill-current" /></button>
            ) : (
              <button className="agent-send" onClick={() => send(input)} disabled={!input.trim() || !info?.codex_path} title="Send (↵)"><ArrowUp className="w-4 h-4" strokeWidth={2.5} /></button>
            )}
          </div>
        </div>
        <p className="text-[10.5px] mt-1.5 px-1 flex items-center gap-1" style={{ color: 'var(--dim)' }}>
          <ShieldCheck className="w-3 h-3" /> Can quarantine and restore. Can’t delete.
        </p>
      </div>
    </aside>
  );
};

/** Model and reasoning effort, straight from Codex's own catalog. "Codex default" passes nothing. */
function ModelPicker() {
  const { models, model, effort, setModel, setEffort } = useStore();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    window.addEventListener('mousedown', h);
    return () => window.removeEventListener('mousedown', h);
  }, [open]);
  const list = models?.models ?? [];
  const chosen = list.find((m) => m.slug === model);
  const codexDefault = list.find((m) => m.slug === models?.default_model);
  const efforts = (chosen ?? codexDefault)?.efforts ?? [];
  const label = chosen ? chosen.display_name : 'Codex default';
  return (
    <div className="relative" ref={ref}>
      <button className="agent-model" onClick={() => setOpen(!open)} title="Model">
        <Brain className="w-3.5 h-3.5" />
        <span className="truncate max-w-[150px]">{label}</span>
        {effort && <span style={{ color: 'var(--dim)' }}>· {effort}</span>}
        <ChevronDown className="w-3 h-3" style={{ color: 'var(--dim)' }} />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div initial={{ opacity: 0, y: 6, scale: 0.98 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: 6, scale: 0.98 }} transition={{ duration: 0.12 }} className="agent-menu">
            <div className="label px-2 pt-1 pb-1.5">Model</div>
            <button className="agent-menu-row" data-on={!model} onClick={() => setModel('')}>
              <span className="flex-1 min-w-0">
                <span className="block text-[12.5px] font-medium">Codex default</span>
                <span className="block text-[11px] truncate" style={{ color: 'var(--dim)' }}>{codexDefault ? `${codexDefault.display_name}${models?.default_effort ? ` · ${models.default_effort}` : ''}, from your Codex settings` : 'Whatever your Codex settings pick'}</span>
              </span>
              {!model && <Check className="w-3.5 h-3.5" />}
            </button>
            <div className="max-h-[260px] overflow-y-auto">
              {list.map((m) => (
                <button key={m.slug} className="agent-menu-row" data-on={m.slug === model} onClick={() => setModel(m.slug)}>
                  <span className="flex-1 min-w-0">
                    <span className="block text-[12.5px] font-medium">{m.display_name}</span>
                    {m.description && <span className="block text-[11px] truncate" style={{ color: 'var(--dim)' }}>{m.description}</span>}
                  </span>
                  {m.slug === model && <Check className="w-3.5 h-3.5" />}
                </button>
              ))}
              {!list.length && <p className="px-2 py-2 text-[11.5px]" style={{ color: 'var(--dim)' }}>Codex didn’t list any models. Disko will use your Codex default.</p>}
            </div>
            {efforts.length > 0 && (
              <>
                <div className="label px-2 pt-2.5 pb-1.5">Thinking</div>
                <div className="flex flex-wrap gap-1 px-1.5 pb-1">
                  <button className="agent-effort" data-on={!effort} onClick={() => setEffort('')}>default</button>
                  {efforts.map((e) => <button key={e} className="agent-effort" data-on={effort === e} onClick={() => setEffort(e)}>{e}</button>)}
                </div>
              </>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** What the agent just moved, with undo and the one action only the user may take: emptying. */
function QuarantineCard({ result }: { result: any }) {
  const journals = useStore((s) => s.journals);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [outcome, setOutcome] = useState<'restored' | 'emptied' | null>(null);
  const journal = journals.find((j) => j.id === result?.batch_id);
  const present = journal?.items.filter((i) => i.present) ?? [];
  const moved: string[] = result?.moved ?? [];
  const home = useStore((s) => s.info?.home ?? '');
  const refresh = () => { const st = useStore.getState(); st.refreshJournals(); st.reloadData(); st.refreshCurrent(); };
  const run = async (kind: 'undo' | 'empty') => {
    if (!journal) return;
    setBusy(kind);
    try {
      if (kind === 'undo') { await api.restore(journal.path); setOutcome('restored'); useStore.getState().toast({ kind: 'success', title: 'Put back where it was' }); }
      else { const freed = await api.purge(journal.path); setOutcome('emptied'); useStore.getState().toast({ kind: 'success', title: `Freed ${formatBytes(freed)}` }); }
      refresh();
    } catch (e) {
      useStore.getState().toast({ kind: 'error', title: kind === 'undo' ? 'Could not restore' : 'Could not empty', detail: errorText(e) });
    } finally { setBusy(null); setConfirm(false); }
  };
  if (!moved.length) return null;
  // Until the journal list catches up, a fresh batch counts as held.
  const state = outcome ?? (!journal || present.length > 0 ? 'held' : journal.purged ? 'emptied' : 'restored');
  return (
    <div className="agent-qcard" data-state={state}>
      <div className="flex items-center gap-2.5">
                <div className="flex-1 min-w-0">
          <div className="text-[12.5px] font-semibold">{state === 'emptied' ? `Emptied · ${result.size} freed` : state === 'restored' ? 'Restored' : `${result.size} in quarantine`}</div>
          <div className="text-[11px]" style={{ color: 'var(--dim)' }}>{moved.length} {moved.length === 1 ? 'item' : 'items'}{state === 'held' ? ' · not deleted yet' : ''}</div>
        </div>
      </div>
      <ul className="mt-2 space-y-0.5">
        {moved.slice(0, 4).map((p) => <li key={p} className="mono text-[10.5px] truncate selectable" style={{ color: 'var(--muted)' }} title={p}>{shortPath(p, home)}</li>)}
        {moved.length > 4 && <li className="text-[10.5px]" style={{ color: 'var(--dim)' }}>and {moved.length - 4} more</li>}
      </ul>
      {state === 'held' && (
        <div className="flex gap-1.5 mt-2.5">
          <button className="btn btn-sm" disabled={!!busy || !journal} onClick={() => run('undo')}><Undo2 className="w-3 h-3" /> Undo</button>
          {confirm ? (
            <button className="btn btn-sm btn-danger" disabled={!!busy} onClick={() => run('empty')}>{busy === 'empty' ? 'Emptying…' : 'Delete for good'}</button>
          ) : (
            <button className="btn btn-sm" disabled={!!busy || !journal} onClick={() => setConfirm(true)}><Trash2 className="w-3 h-3" /> Empty to free {result.size}</button>
          )}
        </div>
      )}
    </div>
  );
}

const Message: React.FC<{ m: ChatMessage; home: string; isStaged: (p: string) => boolean; onToggle: (s: Suggestion) => void; onStageAll: (s: Suggestion[]) => void }> = ({ m, home, isStaged, onToggle, onStageAll }) => {
  const [showActivity, setShowActivity] = useState(false);
  const isUser = m.role === 'user';
  const activity = m.activity ?? [];
  const steps = activity.filter((a) => a.type !== 'tool');
  const tools = activity.filter((a) => a.type === 'tool');
  const running = activity.filter((a) => a.status === 'running');
  const displayText = m.streaming ? extractSuggestions(m.text).clean : m.text;

  if (isUser) {
    return (
      <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} className="flex justify-end my-3">
        <div className="agent-user selectable">{m.text}</div>
      </motion.div>
    );
  }

  return (
    <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} className="my-3">
      {steps.length > 0 && (
        <div className="mb-2">
          <button className="flex items-center gap-1.5 text-[11.5px] text-left max-w-full" style={{ color: 'var(--muted)' }} onClick={() => setShowActivity(!showActivity)}>
            {running.length ? <span className="dots"><span /><span /><span /></span> : null}
            <span className={`truncate min-w-0 ${running.length && running[running.length - 1].type === 'command' ? 'mono text-[11px]' : ''}`}>{running.length ? running[running.length - 1].title : `${steps.length} step${steps.length === 1 ? '' : 's'}`}</span>
            <ChevronRight className={`w-3 h-3 flex-shrink-0 transition-transform ${showActivity ? 'rotate-90' : ''}`} />
          </button>
          <AnimatePresence>
            {showActivity && (
              <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden">
                <div className="mt-1.5 space-y-1 pl-2" style={{ borderLeft: '2px solid var(--line-2)' }}>
                  {steps.map((a) => (
                    <div key={a.id} className="text-[11px]">
                      <span className={`${a.type === 'command' ? 'mono' : ''} break-all selectable`} style={{ color: a.status === 'failed' ? 'var(--danger)' : a.type === 'reasoning' ? 'var(--dim)' : 'var(--muted)' }}>{a.title}</span>
                      {a.detail && a.type === 'command' && <pre className="mono mt-1 text-[10.5px] max-h-36 overflow-auto whitespace-pre-wrap selectable" style={{ color: 'var(--dim)' }}>{a.detail}</pre>}
                    </div>
                  ))}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      )}

      {tools.map((t) => (
        <motion.div key={t.id} initial={{ opacity: 0, scale: 0.97 }} animate={{ opacity: 1, scale: 1 }} className="mb-2">
          <div className="agent-tool" data-status={t.status}>
            {t.status === 'running' ? <span className="dots"><span /><span /><span /></span> : t.status === 'failed' ? <X className="w-3.5 h-3.5" /> : <Check className="w-3.5 h-3.5" />}
            <span className="truncate">{t.title}</span>
          </div>
          {t.tool === 'quarantine' && t.status === 'done' && <QuarantineCard result={t.result} />}
        </motion.div>
      ))}

      {displayText ? (
        <div className="md selectable"><ReactMarkdown remarkPlugins={[remarkGfm]}>{displayText}</ReactMarkdown></div>
      ) : m.streaming && !running.length ? (
        <span className="dots" style={{ color: 'var(--muted)' }}><span /><span /><span /></span>
      ) : null}

      {m.error && <p className="mt-2 text-[12px] selectable" style={{ color: 'var(--danger)' }}>{m.error}</p>}

      {m.suggestions && m.suggestions.length > 0 && (
        <div className="agent-suggest">
          <div className="flex items-center justify-between mb-1.5">
            <span className="label">Needs your OK</span>
            {m.suggestions.some((s) => s.action !== 'command') && (
              <button className="btn btn-sm" onClick={() => onStageAll(m.suggestions!)}>
                <Plus className="w-3 h-3" /> Collect all{m.suggestions.some((s) => s.bytes) ? ` · ${formatBytes(m.suggestions.filter((s) => s.action !== 'command').reduce((a, b) => a + (b.bytes ?? 0), 0))}` : ''}
              </button>
            )}
          </div>
          {m.suggestions.map((s) => {
            const st = isStaged(s.path);
            return (
              <div key={s.path} className="group flex items-start gap-2 py-1.5">
                <span className="w-1.5 h-1.5 rounded-full mt-2 flex-shrink-0" style={{ background: s.confidence === 'high' ? 'var(--ok)' : s.confidence === 'low' ? 'var(--danger)' : 'var(--warn)' }} />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between gap-2">
                    <span className="mono text-[11px] truncate selectable" title={s.path}>{shortPath(s.path, home)}</span>
                    {s.bytes ? <span className="tnum text-[11px] flex-shrink-0" style={{ color: 'var(--muted)' }}>{formatBytes(s.bytes)}</span> : null}
                  </div>
                  {s.reason && <div className="text-[11px] mt-0.5 leading-snug" style={{ color: 'var(--muted)' }}>{s.reason}</div>}
                  {s.action === 'command' && s.command && (
                    <div className="mono text-[10.5px] mt-1 px-2 py-1 rounded-md selectable" style={{ background: 'var(--bg-3)' }}>$ {s.command}</div>
                  )}
                </div>
                {s.action !== 'command' && (
                  <button className="btn btn-ghost btn-icon !p-1 flex-shrink-0" style={st ? { color: 'var(--text)' } : undefined} onClick={() => onToggle(s)}>{st ? <Check className="w-3 h-3" /> : <Plus className="w-3 h-3" />}</button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </motion.div>
  );
};
