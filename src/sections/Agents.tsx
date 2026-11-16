import React, { useEffect, useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronRight, History } from 'lucide-react';
import { useStore, sel, type SelItem } from '../store';
import { age, date, daysSince, splitBytes } from '../lib/format';
import { Glyph, GlassIcon, glyphOf } from '../lib/icons';
import { Checkbox } from '../components/ui/Checkbox';
import type { AgentProject, AgentSession, AgentTool } from '../lib/types';

const sessionItem = (tool: AgentTool, p: AgentProject, s: AgentSession): SelItem =>
  sel.paths(`s:${tool.tool}:${p.name}:${s.id}`, s.title ?? p.name, s.paths, s.bytes, tool.icon);

export const Agents: React.FC = () => {
  const tools = useStore((s) => s.sessions);
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => { if (tools?.length && !tools.some((t) => t.tool === active)) setActive(tools[0].tool); }, [tools, active]);
  const tool = tools?.find((t) => t.tool === active) ?? null;
  const total = (tools ?? []).reduce((a, t) => a + t.bytes, 0);
  const t = splitBytes(total);
  const setMany = useStore((s) => s.setMany);
  const old = useMemo(() => (tool ? tool.projects.flatMap((p) => p.sessions.filter((s) => daysSince(s.last_used) > 30).map((s) => sessionItem(tool, p, s))) : []), [tool]);
  const oldBytes = splitBytes(old.reduce((a, i) => a + i.bytes, 0));

  return (
    <div className="scroll h-full px-7 pb-32">
      <div className="pt-1 pb-6">
        <div className="flex items-baseline gap-2">
          <span className="hero-num text-[64px] leading-[0.9]">{t.value}</span>
          <span className="num unit text-[24px]">{t.unit}</span>
        </div>
        <div className="text-[16px] font-medium muted mt-2.5">kept by your coding agents</div>
      </div>

      <div className="flex gap-3 flex-wrap mb-6">
        {(tools ?? []).map((tl) => {
          const [, color] = glyphOf(tl.icon);
          const s = splitBytes(tl.bytes);
          const on = tl.tool === active;
          return (
            <button
              key={tl.tool}
              onClick={() => setActive(tl.tool)}
              className="relative flex items-center gap-3.5 pl-3 pr-6 py-3 rounded-[22px] text-left transition-all"
              style={{ background: on ? 'var(--panel-2)' : 'var(--panel)', border: `1px solid ${on ? 'var(--line-2)' : 'var(--line)'}`, boxShadow: on ? 'var(--card-shadow), inset 0 0 0 1px var(--line)' : undefined, opacity: on ? 1 : 0.8 }}
            >
              <GlassIcon icon={glyphOf(tl.icon)[0]} color={color} size={46} />
              <div>
                <div className="text-[14px] font-medium muted">{tl.name}</div>
                <div className="flex items-baseline gap-1">
                  <span className="num text-[22px] font-semibold leading-tight">{s.value}</span>
                  <span className="num text-[13px] unit">{s.unit}</span>
                </div>
              </div>
            </button>
          );
        })}
        {!tools && Array.from({ length: 3 }, (_, i) => <div key={i} className="w-[200px] h-[76px] rounded-[22px] shimmer" />)}
      </div>

      {tool && (
        <>
          <div className="flex items-center gap-3 mb-2 px-1">
            <span className="text-[15px] muted">{tool.projects.length} projects</span>
            <div className="flex-1" />
            {old.length > 0 && (
              <button className="btn" onClick={() => setMany(old, true)}>
                <History size={16} /> Select older than a month · <span className="num">{oldBytes.value} {oldBytes.unit}</span>
              </button>
            )}
          </div>
          <div className="flex flex-col gap-1">
            <AnimatePresence initial={false}>
              {tool.projects.slice().sort((a, b) => b.bytes - a.bytes).map((p, i) => (
                <ProjectRow key={p.name + (p.path ?? '')} tool={tool} p={p} max={Math.max(...tool.projects.map((x) => x.bytes))} index={i} />
              ))}
            </AnimatePresence>
          </div>
        </>
      )}
    </div>
  );
};

const ProjectRow: React.FC<{ tool: AgentTool; p: AgentProject; max: number; index: number }> = ({ tool, p, max, index }) => {
  const [open, setOpen] = useState(false);
  const selection = useStore((s) => s.selection);
  const setMany = useStore((s) => s.setMany);
  const items = p.sessions.map((s) => sessionItem(tool, p, s));
  const n = items.filter((i) => selection.has(i.key)).length;
  const [, color] = glyphOf(tool.icon);
  const s = splitBytes(p.bytes);
  const home = useStore((st) => st.boot?.home ?? '');
  return (
    <motion.div layout initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, x: -40, height: 0 }} transition={{ delay: Math.min(0.25, index * 0.02) }} className="rounded-[24px]" style={{ background: open ? 'var(--panel)' : undefined }}>
      <div className="flex items-center gap-4 px-4 py-3 rounded-[24px] row-hover cursor-pointer" onClick={() => setOpen(!open)}>
        <Checkbox checked={n === items.length && n > 0} mixed={n > 0 && n < items.length} onChange={(v) => setMany(items, v)} />
        <ChevronRight size={18} className="muted shrink-0 transition-transform" style={{ transform: open ? 'rotate(90deg)' : undefined }} />
        <Glyph name="folder" size={42} color={color} />
        <div className="flex-1 min-w-0">
          <div className="flex items-baseline gap-2 min-w-0">
            <span className="text-[16px] font-medium truncate">{p.name}</span>
            {p.path && <span className="muted text-[13px] truncate">{p.path.replace(home, '~')}</span>}
          </div>
          <div className="h-[4px] mt-2 max-w-[420px] bar-track">
            <motion.div className="h-full rounded-full" style={{ background: color, opacity: 0.85 }} animate={{ width: `${Math.max(1.5, (p.bytes / max) * 100)}%` }} />
          </div>
        </div>
        <span className="muted text-[14px] w-[110px] text-right shrink-0">{p.sessions.length} {p.sessions.length === 1 ? 'session' : 'sessions'}</span>
        <span className="muted text-[14px] w-[100px] text-right shrink-0">{age(p.last_used)}</span>
        <div className="w-[96px] text-right shrink-0">
          <span className="num font-semibold text-[18px]">{s.value}</span>
          <span className="num text-[13px] unit ml-1">{s.unit}</span>
        </div>
      </div>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden pb-2">
            {p.sessions.map((ss) => <SessionRow key={ss.id} item={sessionItem(tool, p, ss)} s={ss} />)}
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
};

const SessionRow: React.FC<{ item: SelItem; s: AgentSession }> = ({ item, s }) => {
  const picked = useStore((st) => st.selection.has(item.key));
  const toggle = useStore((st) => st.toggle);
  const b = splitBytes(s.bytes);
  return (
    <motion.div layout exit={{ opacity: 0, x: 40, height: 0 }} className="flex items-center gap-4 pl-[74px] pr-4 py-2.5 rounded-2xl row-hover mx-1" onClick={() => toggle(item)} style={{ background: picked ? 'var(--accent-soft)' : undefined }}>
      <Checkbox checked={picked} onChange={() => toggle(item)} size={20} />
      <span className="flex-1 min-w-0 text-[15px] font-medium truncate">{s.title ?? s.id}</span>
      <span className="muted text-[14px] w-[110px] text-right shrink-0">{date(s.started ?? s.last_used)}</span>
      <span className="muted text-[14px] w-[100px] text-right shrink-0">{age(s.last_used)}</span>
      <div className="w-[96px] text-right shrink-0">
        <span className="num font-semibold text-[15px]">{b.value}</span>
        <span className="num text-[12px] unit ml-1">{b.unit}</span>
      </div>
    </motion.div>
  );
};
