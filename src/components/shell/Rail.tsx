import React from 'react';
import { motion } from 'framer-motion';
import { Bot, Brain, LayoutGrid, Orbit, Settings2, Sparkles, type LucideIcon } from 'lucide-react';
import { useStore, type Section } from '../../store';
import { splitBytes } from '../../lib/format';
import { Tip } from '../ui/Tip';

const ITEMS: { id: Section; icon: LucideIcon; label: string }[] = [
  { id: 'space', icon: Orbit, label: 'Space' },
  { id: 'clutter', icon: Sparkles, label: 'Clutter' },
  { id: 'apps', icon: LayoutGrid, label: 'Apps' },
  { id: 'agents', icon: Bot, label: 'Agents' },
  { id: 'memory', icon: Brain, label: 'Memory' },
];

export const Rail: React.FC = () => {
  const section = useStore((s) => s.section);
  const go = useStore((s) => s.go);
  const clutter = useStore((s) => Object.values(s.findings).reduce((a, f) => a + f.bytes, 0));
  const c = splitBytes(clutter);
  return (
    <nav className="drag-region relative z-20 flex flex-col items-center w-[76px] shrink-0 pb-4" style={{ borderRight: '1px solid var(--line)' }}>
      {/* Room for the traffic lights (x16, y18). Empty space here drags the window. */}
      <div className="h-[58px] w-full shrink-0" />
      <div className="flex flex-col items-center gap-2.5">
        {ITEMS.map((it) => (
          <RailButton key={it.id} {...it} active={section === it.id} onClick={() => go(it.id)} badge={it.id === 'clutter' && clutter > 0 ? `${c.value} ${c.unit}` : undefined} />
        ))}
      </div>
      <div className="flex-1 w-full" />
      <RailButton id="settings" icon={Settings2} label="Settings" active={section === 'settings'} onClick={() => go('settings')} />
    </nav>
  );
};

const RailButton: React.FC<{ id: Section; icon: LucideIcon; label: string; active: boolean; onClick: () => void; badge?: string }> = ({ icon: I, label, active, onClick, badge }) => (
  <Tip content={label} side="bottom">
    <button aria-label={label} onClick={onClick} className="relative flex flex-col items-center justify-center w-[52px] rounded-2xl transition-colors" style={{ height: badge ? 60 : 52, color: active ? 'var(--text)' : 'var(--muted)' }}>
      {active && (
        <motion.span
          layoutId="rail-active"
          className="absolute inset-0 rounded-2xl"
          style={{ background: 'var(--panel-3)', boxShadow: 'inset 0 0 0 1px var(--line), inset 0 1px 0 var(--hi)' }}
          transition={{ type: 'spring', stiffness: 420, damping: 40 }}
        />
      )}
      <I size={22} strokeWidth={1.6} className="relative" />
      {badge && <span className="relative num text-[11px] font-medium mt-1" style={{ color: active ? 'var(--text)' : 'var(--accent-text)' }}>{badge}</span>}
    </button>
  </Tip>
);
