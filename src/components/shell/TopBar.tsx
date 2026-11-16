import React from 'react';
import { motion } from 'framer-motion';
import { Check, RotateCw } from 'lucide-react';
import { useStore } from '../../store';
import { bytes, count, splitBytes } from '../../lib/format';
import { Tip } from '../ui/Tip';
import { api } from '../../lib/api';

const TITLES = { space: 'Space', clutter: 'Clutter', apps: 'Apps', agents: 'Agents', memory: 'Memory', settings: 'Settings' } as const;

/** The whole strip is a window handle; only its controls take clicks. */
export const TopBar: React.FC = () => {
  const section = useStore((s) => s.section);
  return (
    <header className="drag-region relative z-10 flex items-center gap-6 h-[68px] shrink-0 pl-7 pr-5">
      <motion.h1 key={section} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="text-[24px] font-semibold tracking-[-0.03em] w-[160px]">
        {TITLES[section]}
      </motion.h1>
      <div className="flex-1 flex justify-center min-w-0">
        <DiskCapsule />
      </div>
      <ScanRing />
    </header>
  );
};

export const DiskCapsule: React.FC = () => {
  const volume = useStore((s) => s.volume);
  const clutter = useStore((s) => Object.values(s.findings).reduce((a, f) => a + f.bytes, 0));
  const go = useStore((s) => s.go);
  if (!volume) return <div className="h-11 w-[440px] rounded-full glass shimmer" />;
  const { total, free } = volume;
  const c = Math.min(clutter, volume.used);
  const usedOther = Math.max(0, volume.used - c);
  const pct = (n: number) => `${(n / total) * 100}%`;
  const f = splitBytes(free);
  const hidden = volume.hidden.reduce((a, h) => a + h.bytes, 0);
  return (
    <div className="flex items-center gap-4 min-w-0">
      <div className="flex items-baseline gap-1.5 shrink-0">
        <span className="num text-[24px] font-medium leading-none">{f.value}</span>
        <span className="num text-[14px] unit">{f.unit} free</span>
      </div>
      <Tip
        side="bottom"
        content={
          <div className="flex flex-col gap-1.5 min-w-[200px]">
            <Legend color="var(--used)" label="In use" value={bytes(usedOther)} />
            <Legend color="var(--accent)" label="Clutter" value={bytes(c)} />
            <Legend color="var(--track)" label="Free" value={bytes(free)} />
            {hidden > 0 && <div className="dim text-[12px] pt-1">{bytes(hidden)} of “in use” is macOS, snapshots and other space no folder shows.</div>}
          </div>
        }
      >
        {/* Segmented solid tones with tiny gaps: in use · clutter · free. */}
        <div className="relative h-[12px] w-[clamp(220px,28vw,420px)] flex gap-[3px] py-[0px]">
          <motion.div className="h-full rounded-l-full shrink-0" style={{ background: 'var(--used)' }} animate={{ width: pct(usedOther) }} transition={{ type: 'spring', stiffness: 120, damping: 26 }} />
          <motion.button
            aria-label="Clutter"
            onClick={() => go('clutter')}
            className="h-full shrink-0 rounded-[2px] transition-[filter] hover:brightness-110"
            style={{ background: 'var(--accent)', boxShadow: 'inset 0 1px 0 rgba(255,255,255,.18)' }}
            animate={{ width: pct(c) }}
            transition={{ type: 'spring', stiffness: 120, damping: 26 }}
          />
          <div className="h-full flex-1 rounded-r-full" style={{ background: 'var(--track)', boxShadow: 'inset 0 0 0 1px var(--line)' }} />
        </div>
      </Tip>
    </div>
  );
};

const Legend: React.FC<{ color: string; label: string; value: string }> = ({ color, label, value }) => (
  <div className="flex items-center gap-2.5">
    <span className="w-2.5 h-2.5 rounded-[3px]" style={{ background: color, boxShadow: 'inset 0 0 0 1px var(--line)' }} />
    <span className="flex-1">{label}</span>
    <span className="num font-semibold">{value}</span>
  </div>
);

export const ScanRing: React.FC = () => {
  const p = useStore((s) => s.progress);
  const rescan = () => api.rescan();
  const done = !p || p.phase === 'done';
  const frac = p?.fraction ?? 0;
  const R = 13;
  const C = 2 * Math.PI * R;
  return (
    <Tip
      side="bottom"
      content={
        done ? (
          <span>Up to date. Disko keeps watching for changes.<br /><span className="dim">Click to look again from scratch.</span></span>
        ) : (
          <span>
            <span className="num font-semibold">{Math.round(frac * 100)}%</span> · {count(p!.files)} files
            <br />
            <span className="dim break-all">{p!.current.replace(/^\/Users\/[^/]+/, '~')}</span>
          </span>
        )
      }
    >
      <button className="no-drag relative w-10 h-10 flex items-center justify-center rounded-full row-hover" aria-label={done ? 'Scan again' : 'Scanning'} onClick={() => done && rescan()}>
        <svg width="34" height="34" viewBox="0 0 34 34" className="absolute">
          <circle cx="17" cy="17" r={R} fill="none" stroke="var(--track)" strokeWidth="2" />
          <motion.circle
            cx="17" cy="17" r={R} fill="none" stroke="var(--accent-text)" strokeWidth="2" strokeLinecap="round"
            transform="rotate(-90 17 17)" strokeDasharray={C}
            animate={{ strokeDashoffset: C * (1 - (done ? 1 : Math.max(0.04, frac))) }}
            transition={{ type: 'spring', stiffness: 80, damping: 20 }}
          />
        </svg>
        {done ? <Check size={14} strokeWidth={2.4} style={{ color: 'var(--accent-text)' }} /> : <RotateCw size={13} className="muted" style={{ animation: 'spin 1.4s linear infinite' }} />}
      </button>
    </Tip>
  );
};
