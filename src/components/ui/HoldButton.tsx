import React, { useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';

/**
 * For the one thing Disko can't undo (emptying the Trash): press and hold until it fills.
 * No dialog. Releasing early does nothing.
 */
export const HoldButton: React.FC<{ onDone: () => void; children: React.ReactNode; ms?: number; className?: string; disabled?: boolean }> = ({ onDone, children, ms = 1100, className, disabled }) => {
  const [p, setP] = useState(0);
  const start = useRef<number | null>(null);
  const raf = useRef(0);
  const stop = () => { start.current = null; cancelAnimationFrame(raf.current); setP(0); };
  const tick = (now: number) => {
    if (start.current === null) return;
    const v = Math.min(1, (now - start.current) / ms);
    setP(v);
    if (v >= 1) { stop(); onDone(); return; }
    raf.current = requestAnimationFrame(tick);
  };
  useEffect(() => () => cancelAnimationFrame(raf.current), []);
  const begin = () => { if (disabled) return; start.current = performance.now(); raf.current = requestAnimationFrame(tick); };
  return (
    <button
      className={`btn btn-danger relative overflow-hidden ${className ?? ''}`}
      disabled={disabled}
      onPointerDown={begin}
      onPointerUp={stop}
      onPointerLeave={stop}
      onKeyDown={(e) => { if ((e.key === ' ' || e.key === 'Enter') && start.current === null && !e.repeat) begin(); }}
      onKeyUp={stop}
    >
      <motion.span className="absolute inset-0 origin-left" style={{ background: 'rgba(255,255,255,.22)', scaleX: p }} />
      <span className="relative inline-flex items-center gap-2">{p > 0 ? 'Keep holding…' : children}</span>
    </button>
  );
};
