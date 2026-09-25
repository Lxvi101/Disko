import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowUp, Square, X, ChevronRight, Plus, Check, Trash2 } from 'lucide-react';
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

export const Assistant: React.FC = () => {
  const { messages, generating, model, effort, models, setModel, setEffort, setAssistantOpen, currentPath, currentItems, staged, page, info, clearChat, toast, stageMany, isStaged, toggleStage, advanced } = useStore();
  const [input, setInput] = useState('');
  const listRef = useRef<HTMLDivElement>(null);
  const activeReq = useRef<string | null>(null);
  const [showModels, setShowModels] = useState(false);
  const currentModel = useMemo(() => models?.models.find((m) => m.slug === model), [models, model]);

  useEffect(() => {
    let un: (() => void) | null = null;
    onCodexEvent((p) => {
      const st = useStore.getState();
      if (!st.messages.some((m) => m.id === p.request_id)) return;
      const ev = p.event;
      const type: string = ev.type;
      if (type !== 'item.started' && type !== 'item.completed' && type !== 'item.updated') return;
      const item = ev.item ?? {};
      const itemType: string = item.type;
      const id = String(item.id ?? uid());
      st.updateMessage(p.request_id, (m) => {
        const activity = [...(m.activity ?? [])];
        const idx = activity.findIndex((a) => a.id === id);
        let entry: ActivityItem | null = null;
        if (itemType === 'command_execution') {
          const done = type === 'item.completed';
          entry = { id, type: 'command', title: summarizeCommand(String(item.command ?? '')), detail: String(item.aggregated_output ?? '').slice(0, 4000), status: done ? (item.exit_code === 0 || item.exit_code == null ? 'done' : 'failed') : 'running' };
        } else if (itemType === 'reasoning') {
          entry = { id, type: 'reasoning', title: String(item.text ?? item.summary ?? 'Thinking').slice(0, 400), status: 'done' };
        } else if (itemType === 'error') {
          entry = { id, type: 'error', title: String(item.message ?? 'Error'), status: 'failed' };
        } else if (itemType === 'agent_message' && type === 'item.completed') {
          const text = String(item.text ?? '');
          return { text: m.text ? `${m.text}\n\n${text}` : text };
        }
        if (!entry) return {};
        if (idx >= 0) activity[idx] = entry;
        else activity.push(entry);
        return { activity };
      });
    }).then((u) => (un = u));
    return () => un?.();
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

  const quick = useMemo(() => {
    const st = useStore.getState();
    const folder = !currentPath || currentPath === st.rootPath ? 'my home folder' : currentPath.split('/').filter(Boolean).pop() ?? 'this folder';
    const base = [
      { label: 'What can I safely free up?', q: 'What can I safely free up right now? Rank by size and safety, and list exact paths in a disko-actions block.' },
      { label: `What is in ${folder}?`, q: `Explain what is taking up space in ${currentPath} and which parts are safe to remove.` },
      { label: 'Developer leftovers', q: 'Find leftover developer caches, old node_modules, virtualenvs, simulators and build outputs I am probably not using anymore.' },
    ];
    if (staged.length) base.unshift({ label: `Review the ${staged.length} collected`, q: 'Review the items I collected for cleanup. Flag anything risky and say what each one costs to recreate.' });
    return base;
  }, [currentPath, staged.length]);

  const stageSuggestions = (all: Suggestion[]) => {
    const s = all.filter((x) => x.action !== 'command');
    if (s.length === 0) return;
    stageMany(s.map((x) => ({ path: x.path, name: x.path.split('/').filter(Boolean).pop() ?? x.path, kind: 'directory', total: x.bytes ?? 0, category: 'review', reason: x.reason, source: 'assistant' as const })));
    toast({ kind: 'success', title: `Collected ${s.length} item${s.length === 1 ? '' : 's'}` });
  };

  return (
    <aside className="h-full flex flex-col" style={{ borderLeft: '1px solid var(--line)' }}>
      <div className="h-[52px] flex items-center px-4 flex-shrink-0 drag-region">
        <span className="text-[13px] font-semibold">Disko</span>
        {advanced && (
          <button className="no-drag ml-2 text-[11.5px] hover:text-white transition-colors" style={{ color: 'var(--muted)' }} onClick={() => setShowModels(!showModels)}>
            {currentModel?.display_name ?? model ?? 'default'} · {effort || 'default'}
          </button>
        )}
        <span className="flex-1" />
        {messages.length > 0 && (
          <button className="no-drag btn btn-ghost btn-icon" title="Clear" onClick={clearChat} disabled={!!generating}><Trash2 className="w-3.5 h-3.5" /></button>
        )}
        <button className="no-drag btn btn-ghost btn-icon" onClick={() => setAssistantOpen(false)} title="Hide"><X className="w-4 h-4" /></button>
      </div>

      <AnimatePresence>
        {showModels && advanced && (
          <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden flex-shrink-0" style={{ borderBottom: '1px solid var(--line)' }}>
            <div className="px-3 pb-3">
              <div className="label px-1 py-1.5">Model</div>
              {(models?.models ?? []).map((m) => (
                <button key={m.slug} onClick={() => setModel(m.slug)} className="w-full text-left px-2 py-1.5 rounded-md row-hover flex items-center gap-2" style={{ background: m.slug === model ? 'var(--bg-3)' : undefined }}>
                  <span className="text-[12.5px]">{m.display_name}</span>
                  <span className="text-[11px] truncate flex-1" style={{ color: 'var(--dim)' }}>{m.description}</span>
                  {m.slug === models?.default_model && <span className="chip">default</span>}
                </button>
              ))}
              <div className="label px-1 py-1.5 mt-1">Reasoning</div>
              <div className="flex flex-wrap gap-1 px-1">
                {(currentModel?.efforts ?? ['low', 'medium', 'high']).map((e) => (
                  <button key={e} onClick={() => setEffort(e)} className="chip" style={{ color: effort === e ? 'var(--text)' : 'var(--muted)', background: effort === e ? 'var(--bg-3)' : undefined }}>{e}</button>
                ))}
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div ref={listRef} className="flex-1 min-h-0 overflow-y-auto px-4 py-2">
        {messages.length === 0 && (
          <div className="pt-4">
            <p className="text-[12.5px] leading-relaxed" style={{ color: 'var(--muted)' }}>
              Ask about anything on your disk. Disko runs Codex in a read-only sandbox with your scan, so it can look inside folders and hand you a plan you can collect in one click.
            </p>
            <div className="flex flex-col items-start gap-1 mt-4">
              {quick.map((q) => (
                <button key={q.label} className="text-left text-[12.5px] px-2 py-1 -mx-2 rounded-md row-hover" onClick={() => send(q.q)}>{q.label}</button>
              ))}
            </div>
          </div>
        )}
        {messages.map((m) => (
          <Message key={m.id} m={m} home={info?.home ?? ''} isStaged={isStaged} onToggle={(s) => toggleStage({ path: s.path, name: s.path.split('/').pop() ?? s.path, kind: 'directory', total: s.bytes ?? 0, category: 'review', reason: s.reason, source: 'assistant' })} onStageAll={stageSuggestions} />
        ))}
      </div>

      {messages.length > 0 && !generating && (
        <div className="px-4 pb-2 flex gap-1.5 overflow-x-auto flex-shrink-0" style={{ scrollbarWidth: 'none' }}>
          {quick.slice(0, 3).map((q) => (
            <button key={q.label} className="btn btn-sm" onClick={() => send(q.q)}>{q.label}</button>
          ))}
        </div>
      )}

      <div className="p-3 flex-shrink-0">
        <div className="flex items-end gap-2 rounded-xl px-3 py-2" style={{ background: 'var(--bg-2)', border: '1px solid var(--line)' }}>
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(input); } }}
            rows={Math.min(5, Math.max(1, input.split('\n').length))}
            placeholder={info?.codex_path ? 'Ask Disko…' : 'Codex CLI not found'}
            disabled={!info?.codex_path}
            className="flex-1 bg-transparent outline-none resize-none text-[13px] py-0.5 selectable"
            style={{ color: 'var(--text)' }}
          />
          {generating ? (
            <button className="btn btn-icon !p-1.5" onClick={stop} title="Stop"><Square className="w-3 h-3 fill-current" /></button>
          ) : (
            <button className="btn btn-primary btn-icon !p-1.5" onClick={() => send(input)} disabled={!input.trim() || !info?.codex_path} title="Send"><ArrowUp className="w-3.5 h-3.5" /></button>
          )}
        </div>
        <div className="flex justify-between px-1 mt-1.5 text-[10.5px]" style={{ color: 'var(--dim)' }}>
          <span>{page === 'explore' ? shortPath(currentPath, info?.home ?? '') || '~' : page}{staged.length ? ` · ${staged.length} collected` : ''}</span>
          <span>{info?.codex_version ? 'read-only' : ''}</span>
        </div>
      </div>
    </aside>
  );
};

const Message: React.FC<{ m: ChatMessage; home: string; isStaged: (p: string) => boolean; onToggle: (s: Suggestion) => void; onStageAll: (s: Suggestion[]) => void }> = ({ m, home, isStaged, onToggle, onStageAll }) => {
  const [showActivity, setShowActivity] = useState(false);
  const isUser = m.role === 'user';
  const activity = m.activity ?? [];
  const running = activity.filter((a) => a.status === 'running');
  const displayText = m.streaming ? extractSuggestions(m.text).clean : m.text;

  if (isUser) {
    return (
      <motion.div initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="flex justify-end my-3">
        <div className="max-w-[88%] rounded-xl px-3 py-2 text-[13px] leading-relaxed selectable" style={{ background: 'var(--bg-3)' }}>{m.text}</div>
      </motion.div>
    );
  }

  return (
    <motion.div initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="my-3">
      {activity.length > 0 && (
        <div className="mb-1.5">
          <button className="flex items-center gap-1.5 text-[11.5px] text-left max-w-full" style={{ color: 'var(--muted)' }} onClick={() => setShowActivity(!showActivity)}>
            {running.length ? <span className="dots"><span /><span /><span /></span> : null}
            <span className={`truncate min-w-0 ${running.length ? 'mono text-[11px]' : ''}`}>{running.length ? summarizeCommand(running[running.length - 1].title) : `${activity.length} step${activity.length === 1 ? '' : 's'}`}</span>
            <ChevronRight className={`w-3 h-3 flex-shrink-0 transition-transform ${showActivity ? 'rotate-90' : ''}`} />
          </button>
          <AnimatePresence>
            {showActivity && (
              <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden">
                <div className="mt-1.5 space-y-1 pl-2" style={{ borderLeft: '2px solid var(--line-2)' }}>
                  {activity.map((a) => (
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

      {displayText ? (
        <div className="md selectable"><ReactMarkdown remarkPlugins={[remarkGfm]}>{displayText}</ReactMarkdown></div>
      ) : m.streaming ? (
        <span className="dots" style={{ color: 'var(--muted)' }}><span /><span /><span /></span>
      ) : null}

      {m.error && <p className="mt-2 text-[12px] selectable" style={{ color: 'var(--danger)' }}>{m.error}</p>}

      {m.suggestions && m.suggestions.length > 0 && (
        <div className="mt-3 pt-3" style={{ borderTop: '1px solid var(--line)' }}>
          <div className="flex items-center justify-between mb-1.5">
            <span className="label">Suggested</span>
            {m.suggestions.some((s) => s.action !== 'command') && (
              <button className="btn btn-sm" onClick={() => onStageAll(m.suggestions!)}>
                Collect all{m.suggestions.some((s) => s.bytes) ? ` · ${formatBytes(m.suggestions.filter((s) => s.action !== 'command').reduce((a, b) => a + (b.bytes ?? 0), 0))}` : ''}
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
