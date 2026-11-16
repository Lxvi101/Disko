import React from 'react';
import { motion } from 'framer-motion';
import { Moon, RotateCw, ShieldCheck, Sun, SunMoon } from 'lucide-react';
import { useStore, type Theme } from '../store';
import { api } from '../lib/api';

const THEMES: { id: Theme; icon: typeof Sun; label: string }[] = [
  { id: 'system', icon: SunMoon, label: 'Auto' },
  { id: 'dark', icon: Moon, label: 'Dark' },
  { id: 'light', icon: Sun, label: 'Light' },
];

export const Settings: React.FC = () => {
  const theme = useStore((s) => s.theme);
  const setTheme = useStore((s) => s.setTheme);
  const boot = useStore((s) => s.boot);
  return (
    <div className="scroll h-full px-7 pb-24">
      <div className="flex flex-col gap-4 max-w-[640px]">
        <Row title="Look">
          <div className="glass rounded-full p-1 flex gap-1">
            {THEMES.map((t) => (
              <button key={t.id} onClick={() => setTheme(t.id)} className="relative h-10 px-4 rounded-full flex items-center gap-2 text-[14px] font-semibold" style={{ color: theme === t.id ? 'white' : 'var(--muted)' }}>
                {theme === t.id && <motion.span layoutId="theme-pill" className="absolute inset-0 rounded-full" style={{ background: 'linear-gradient(135deg,#9a7bff,#5b6cff)' }} />}
                <t.icon size={17} className="relative" /><span className="relative">{t.label}</span>
              </button>
            ))}
          </div>
        </Row>
        <Row title="Full Disk Access">
          {boot?.full_disk_access ? (
            <span className="flex items-center gap-2 font-semibold" style={{ color: 'var(--ok)' }}><ShieldCheck size={20} /> On</span>
          ) : (
            <button className="btn btn-primary" onClick={() => api.openFullDiskAccess()}>Open Settings</button>
          )}
        </Row>
        <Row title="Look at everything again">
          <button className="btn" onClick={() => api.rescan()}><RotateCw size={16} /> Scan again</button>
        </Row>
        <div className="muted text-[14px] px-2 pt-2">Disko {boot?.version ?? ''}</div>
      </div>
    </div>
  );
};

const Row: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <div className="glass rounded-[24px] flex items-center gap-4 px-5 py-4">
    <div className="flex-1 text-[17px] font-semibold">{title}</div>
    {children}
  </div>
);
