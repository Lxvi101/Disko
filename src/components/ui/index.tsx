import React, { useEffect, useId, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X } from 'lucide-react';
import { catMeta } from '../../lib/format';

export const CategoryChip: React.FC<{ category: string; className?: string; small?: boolean }> = ({ category, className = '', small }) => {
  const m = catMeta(category);
  return (
    <span className={`chip ${className}`} style={{ color: m.text, fontSize: small ? 10 : undefined }}>
      <span className="w-1.5 h-1.5 rounded-full" style={{ background: m.color }} />
      {m.label}
    </span>
  );
};

export const Spinner: React.FC<{ size?: number; className?: string }> = ({ size = 14, className = '' }) => (
  <svg className={`animate-spin ${className}`} width={size} height={size} viewBox="0 0 24 24" fill="none">
    <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.2" strokeWidth="3" />
    <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
  </svg>
);

export const Modal: React.FC<{
  open: boolean;
  onClose: () => void;
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  icon?: React.ReactNode;
  width?: number;
  children: React.ReactNode;
  footer?: React.ReactNode;
  locked?: boolean;
}> = ({ open, onClose, title, subtitle, width = 520, children, footer, locked }) => {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  const lockedRef = useRef(locked);
  closeRef.current = onClose;
  lockedRef.current = locked;
  const titleId = useId();
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const timer = setTimeout(() => ref.current?.focus(), 0);
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault(); e.stopImmediatePropagation();
        if (!lockedRef.current) closeRef.current();
      }
      if (e.key === 'Tab') {
        const controls = Array.from(ref.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex="0"]') ?? []);
        const first = controls[0], last = controls[controls.length - 1];
        if (!first) { e.preventDefault(); return; }
        if (e.shiftKey && (document.activeElement === first || document.activeElement === ref.current)) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && (document.activeElement === last || document.activeElement === ref.current)) { e.preventDefault(); first.focus(); }
      }
    };
    window.addEventListener('keydown', key, true);
    return () => { clearTimeout(timer); window.removeEventListener('keydown', key, true); if (previous?.isConnected) previous.focus(); };
  }, [open]);
  return (
  <AnimatePresence>
    {open && (
      <motion.div className="fixed inset-0 z-[80] flex items-center justify-center p-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.15 }}>
        <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.55)' }} onClick={() => !locked && onClose()} />
        <motion.div
          ref={ref} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}
          className="panel-raised relative overflow-hidden flex flex-col max-h-[85vh] outline-none"
          style={{ width: '100%', maxWidth: width }}
          initial={{ opacity: 0, y: 12, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8, scale: 0.98 }}
          transition={{ type: 'spring', stiffness: 420, damping: 34 }}
        >
          <div className="px-5 pt-4 pb-3 flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h2 id={titleId} className="text-[14px] font-semibold leading-tight">{title}</h2>
              {subtitle && <p className="text-[12px] mt-0.5" style={{ color: 'var(--muted)' }}>{subtitle}</p>}
            </div>
            <button aria-label="Close dialog" className="btn btn-ghost btn-icon -mr-2 -mt-1" onClick={onClose} disabled={locked}>
              <X className="w-4 h-4" />
            </button>
          </div>
          <div className="px-5 pb-4 overflow-y-auto flex-1">{children}</div>
          {footer && (
            <div className="px-5 py-3 flex items-center justify-end gap-2" style={{ borderTop: '1px solid var(--line)' }}>
              {footer}
            </div>
          )}
        </motion.div>
      </motion.div>
    )}
  </AnimatePresence>
);
};

export const Empty: React.FC<{ icon: React.ReactNode; title: string; body?: React.ReactNode; action?: React.ReactNode }> = ({ icon, title, body, action }) => (
  <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} className="flex flex-col items-center justify-center text-center h-full py-16 px-6">
    <div className="mb-3" style={{ color: 'var(--dim)' }}>{icon}</div>
    <h3 className="text-[14px] font-semibold">{title}</h3>
    {body && <p className="text-[12.5px] mt-1.5 max-w-sm leading-relaxed" style={{ color: 'var(--muted)' }}>{body}</p>}
    {action && <div className="mt-4">{action}</div>}
  </motion.div>
);

export const SegmentedControl = <T extends string>({ value, onChange, options }: { value: T; onChange: (v: T) => void; options: { value: T; label: React.ReactNode; title?: string }[] }) => (
  <div className="relative inline-flex items-center gap-0.5">
    {options.map((o) => {
      const active = o.value === value;
      return (
        <button key={o.value} title={o.title} onClick={() => onChange(o.value)} className="relative px-2.5 py-1 rounded-md text-[12px] font-medium transition-colors" style={{ color: active ? 'var(--text)' : 'var(--muted)' }}>
          {active && <motion.span layoutId="segment-bg" className="absolute inset-0 rounded-md" style={{ background: 'var(--bg-3)' }} transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
          <span className="relative flex items-center gap-1.5">{o.label}</span>
        </button>
      );
    })}
  </div>
);

export const Switch: React.FC<{ on: boolean; onChange: (v: boolean) => void; label?: React.ReactNode }> = ({ on, onChange, label }) => (
  <button className="flex items-center gap-2 text-[12px]" style={{ color: on ? 'var(--text)' : 'var(--muted)' }} onClick={() => onChange(!on)}>
    {label}
    <span className={`switch ${on ? 'on' : ''}`} />
  </button>
);
