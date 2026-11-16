import React from 'react';
import { Check, Minus } from 'lucide-react';

/** Round, generous check target. `mixed` shows a dash (some children selected). */
export const Checkbox: React.FC<{ checked: boolean; mixed?: boolean; onChange: (v: boolean) => void; color?: string; size?: number; disabled?: boolean; title?: string }> = ({ checked, mixed, onChange, color = 'var(--accent)', size = 22, disabled, title }) => (
  <span
    role="checkbox"
    aria-checked={mixed ? 'mixed' : checked}
    aria-disabled={disabled}
    title={title}
    tabIndex={disabled ? -1 : 0}
    onClick={(e) => { e.stopPropagation(); if (!disabled) onChange(!checked); }}
    onKeyDown={(e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); if (!disabled) onChange(!checked); } }}
    className="inline-flex items-center justify-center shrink-0 transition-all"
    style={{
      width: size, height: size, borderRadius: 999,
      background: checked || mixed ? color : 'transparent',
      border: checked || mixed ? `1.5px solid ${color}` : '1.5px solid var(--line-2)',
      boxShadow: checked || mixed ? 'inset 0 1px 0 rgba(255,255,255,.2)' : undefined,
      opacity: disabled ? 0.35 : 1,
      color: 'white',
    }}
  >
    {mixed ? <Minus size={size * 0.62} strokeWidth={3} /> : checked ? <Check size={size * 0.62} strokeWidth={3} /> : null}
  </span>
);
