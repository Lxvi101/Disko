import React, { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Bot, Check, Clock, Globe2, Play, Plus, Shield, User, X, Users, Plug } from 'lucide-react';
import { useStore } from '../store';
import { api } from '../lib/api';
import { CAT, catOf } from '../lib/categories';
import { age, bytes } from '../lib/format';
import { Glyph } from '../lib/icons';
import { Tip } from '../components/ui/Tip';
import type { MemoryOp, Routine } from '../lib/types';

const SOURCE = {
  user: { icon: User, label: 'You', color: '#8d7bff' },
  agent: { icon: Bot, label: 'An agent', color: '#2fd3e6' },
  community: { icon: Users, label: 'The community', color: '#3ddc97' },
} as const;

const update = (op: MemoryOp) => api.memoryUpdate(op).then((m) => useStore.setState({ memory: m }));

export const Memory: React.FC = () => {
  const memory = useStore((s) => s.memory);
  const home = useStore((s) => s.boot?.home ?? '');
  const tilde = (p: string) => (home ? p.replace(home, '~') : p);
  if (!memory) return <div className="p-7"><div className="h-40 rounded-[28px] shimmer" /></div>;
  return (
    <div className="scroll h-full px-7 pb-32">
      <div className="gap-5 [&>*]:mb-5 [&>*]:break-inside-avoid" style={{ columns: '2 440px' }}>
        <Connect />
        <Card title="Routines" icon={Clock} color="#ff8a3d">
          {!memory.routines.length && <Empty>Turn on the clock on any clutter card.</Empty>}
          <AnimatePresence initial={false}>
            {memory.routines.map((r) => <RoutineRow key={r.id} r={r} />)}
          </AnimatePresence>
        </Card>
        <Card title="Always keep" icon={Shield} color="#3ddc97" extra={<AddKeep />}>
          {!memory.keep.length && <Empty>Nothing yet. Disko never touches what you list here.</Empty>}
          <AnimatePresence initial={false}>
            {memory.keep.map((k) => (
              <Line key={k.id} source={k.source} onRemove={() => update({ type: 'unkeep', id: k.id })}>
                <div className="text-[15px] font-semibold truncate">{k.note ?? tilde(k.pattern).split('/').pop()}</div>
                <div className="muted text-[13px] truncate">{tilde(k.pattern)}</div>
              </Line>
            ))}
          </AnimatePresence>
        </Card>
        <Card title="What Disko learned" icon={Bot} color="#2fd3e6">
          <AnimatePresence initial={false}>
            {memory.rules.map((r) => {
              const cat = catOf(r.category) ?? CAT.cache;
              return (
                <Line key={r.id} source={r.source} lead={<Glyph name="x" icon={cat.icon} color={cat.color} size={38} />} onRemove={() => update({ type: 'remove-rule', id: r.id })}>
                  <Tip content={r.why} className="block">
                    <div className="text-[15px] font-semibold truncate">{r.title}</div>
                    <div className="muted text-[13px] truncate">{tilde(r.pattern)}</div>
                  </Tip>
                </Line>
              );
            })}
          </AnimatePresence>
          <Community />
        </Card>
      </div>
    </div>
  );
};

const Card: React.FC<{ title: string; icon: typeof Clock; color: string; children: React.ReactNode; extra?: React.ReactNode }> = ({ title, icon: I, color, children, extra }) => (
  <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="glass rounded-[28px] p-5 flex flex-col gap-1 min-w-0">
    <div className="flex items-center gap-3 mb-3">
      <span className="w-10 h-10 rounded-[14px] flex items-center justify-center" style={{ background: `linear-gradient(145deg, ${color}, ${color}aa)` }}><I size={21} color="white" /></span>
      <h2 className="text-[20px] font-bold flex-1">{title}</h2>
      {extra}
    </div>
    {children}
  </motion.section>
);

const Empty: React.FC<{ children: React.ReactNode }> = ({ children }) => <div className="muted text-[15px] px-2 py-3">{children}</div>;

const Line: React.FC<{ source: keyof typeof SOURCE; children: React.ReactNode; onRemove: () => void; lead?: React.ReactNode }> = ({ source, children, onRemove, lead }) => {
  const s = SOURCE[source];
  return (
    <motion.div layout initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0, x: 30, height: 0 }} className="group flex items-center gap-3 px-2 py-2 rounded-2xl row-hover">
      {lead}
      <div className="flex-1 min-w-0">{children}</div>
      <Tip content={`Added by ${s.label.toLowerCase()}`}>
        <span className="w-8 h-8 rounded-full flex items-center justify-center" style={{ background: `${s.color}22`, color: s.color }}><s.icon size={16} /></span>
      </Tip>
      <button className="btn btn-ghost btn-icon !w-8 !h-8 opacity-0 group-hover:opacity-100" aria-label="Remove" onClick={onRemove}><X size={16} /></button>
    </motion.div>
  );
};

const RoutineRow: React.FC<{ r: Routine }> = ({ r }) => {
  const cat = catOf(r.select.categories?.[0]);
  const toast = useStore((s) => s.toast);
  const [running, setRunning] = useState(false);
  const run = async () => {
    setRunning(true);
    try {
      const res = await api.routineRun(r.id);
      toast({ kind: 'freed', title: res.freed ? `${bytes(res.freed)} freed` : 'Already clean' });
      useStore.getState().refreshMemory();
      useStore.getState().refreshClutter();
    } finally { setRunning(false); }
  };
  return (
    <motion.div layout exit={{ opacity: 0, height: 0 }} className="group flex items-center gap-3 px-2 py-2 rounded-2xl row-hover">
      <Glyph name="x" icon={cat?.icon ?? Clock} color={cat?.color ?? '#ff8a3d'} size={38} />
      <div className="flex-1 min-w-0">
        <div className="text-[15px] font-semibold truncate">{r.title}</div>
        <div className="muted text-[13px] truncate">Every {r.every_days === 1 ? 'day' : `${r.every_days} days`} · {r.last_run ? `${age(r.last_run)}${r.last_freed ? `, freed ${bytes(r.last_freed)}` : ''}` : 'not run yet'}</div>
      </div>
      <Tip content="Run now"><button className="btn btn-ghost btn-icon !w-8 !h-8" disabled={running} aria-label="Run now" onClick={run}><Play size={15} /></button></Tip>
      <Switch on={r.enabled} onChange={(v) => update({ type: 'set-routine', routine: { ...r, enabled: v } })} color={cat?.color} />
      <button className="btn btn-ghost btn-icon !w-8 !h-8 opacity-0 group-hover:opacity-100" aria-label="Remove" onClick={() => update({ type: 'remove-routine', id: r.id })}><X size={16} /></button>
    </motion.div>
  );
};

export const Switch: React.FC<{ on: boolean; onChange: (v: boolean) => void; color?: string }> = ({ on, onChange, color = 'var(--accent)' }) => (
  <button role="switch" aria-checked={on} onClick={() => onChange(!on)} className="relative w-[46px] h-[28px] rounded-full transition-colors shrink-0" style={{ background: on ? color : 'var(--panel-3)' }}>
    <motion.span className="absolute top-[3px] w-[22px] h-[22px] rounded-full bg-white" style={{ boxShadow: '0 2px 6px rgba(0,0,0,.3)' }} animate={{ left: on ? 21 : 3 }} transition={{ type: 'spring', stiffness: 600, damping: 34 }} />
  </button>
);

const AddKeep: React.FC = () => {
  const [open, setOpen] = useState(false);
  const [v, setV] = useState('');
  const submit = () => { if (v.trim()) update({ type: 'keep', pattern: v.trim() }); setV(''); setOpen(false); };
  return open ? (
    <input
      autoFocus value={v} onChange={(e) => setV(e.target.value)} onBlur={() => !v && setOpen(false)}
      onKeyDown={(e) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') setOpen(false); }}
      placeholder="~/path/to/keep" className="h-9 px-3 rounded-full text-[14px] outline-none w-[210px] select-text"
      style={{ background: 'var(--panel-2)', border: '1px solid var(--line-2)', color: 'var(--text)' }}
    />
  ) : (
    <Tip content="Add a path"><button className="btn btn-icon !w-9 !h-9" aria-label="Add a path" onClick={() => setOpen(true)}><Plus size={18} /></button></Tip>
  );
};

const Community: React.FC = () => {
  const c = useStore((s) => s.memory?.community);
  if (!c) return null;
  return (
    <div className="mt-3 flex items-center gap-3 px-3 py-3 rounded-2xl" style={{ background: 'var(--panel-2)' }}>
      <Globe2 size={22} style={{ color: '#3ddc97' }} />
      <div className="flex-1 min-w-0">
        <div className="text-[15px] font-semibold">Share anonymously</div>
        <div className="muted text-[13px]">Share the rules you teach Disko · {c.rules} rules from others in use</div>
      </div>
      <Switch on={c.enabled} onChange={(v) => update({ type: 'community', enabled: v })} color="#3ddc97" />
    </div>
  );
};

const CLIENTS = [
  { id: 'claude-code', icon: 'claude' },
  { id: 'codex', icon: 'codex' },
  { id: 'cursor', icon: 'cursor' },
];

const Connect: React.FC = () => {
  const mcp = useStore((s) => s.mcp);
  const [busy, setBusy] = useState<string | null>(null);
  const connect = async (id: string) => {
    setBusy(id);
    try { useStore.setState({ mcp: await api.mcpInstall(id) }); } catch (e) { useStore.getState().toast({ kind: 'error', title: 'Couldn’t connect', sub: String(e) }); }
    setBusy(null);
  };
  return (
    <motion.section className="rounded-[28px] p-6 relative overflow-hidden" style={{ background: 'linear-gradient(130deg, rgba(141,123,255,.28), rgba(76,201,255,.14) 60%, rgba(255,95,162,.12))', border: '1px solid rgba(141,123,255,.35)' }}>
      <div className="flex items-center gap-3">
        <span className="w-10 h-10 rounded-[14px] flex items-center justify-center" style={{ background: 'linear-gradient(145deg,#9a7bff,#3fb5ff)' }}><Plug size={21} color="white" /></span>
        <h2 className="text-[20px] font-bold">Connect your agents</h2>
      </div>
      <div className="muted text-[15px] mt-2 mb-4">They can ask Disko what’s taking space, clean, and teach it new rules.</div>
      <div className="flex flex-wrap gap-3">
        {CLIENTS.map((c) => {
          const st = mcp?.find((m) => m.id === c.id);
          const name = st?.name ?? c.id;
          return (
            <button
              key={c.id}
              disabled={!st?.present || st.connected || busy === c.id}
              onClick={() => connect(c.id)}
              className="flex items-center gap-3 pl-2 pr-4 h-[56px] rounded-[18px] transition-all"
              style={{ background: 'var(--panel-2)', border: '1px solid var(--line-2)', opacity: st && !st.present ? 0.45 : 1 }}
            >
              <Glyph name={c.icon} size={40} />
              <div className="text-left">
                <div className="text-[15px] font-semibold">{name}</div>
                <div className="text-[13px] font-medium" style={{ color: st?.connected ? 'var(--ok)' : 'var(--muted)' }}>
                  {st?.connected ? <span className="inline-flex items-center gap-1"><Check size={13} strokeWidth={3} /> Connected</span> : !st ? '…' : st.present ? (busy === c.id ? 'Connecting…' : 'Connect') : 'Not installed'}
                </div>
              </div>
            </button>
          );
        })}
      </div>
    </motion.section>
  );
};
