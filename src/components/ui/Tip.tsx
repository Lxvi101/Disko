import React, { useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/** A soft hover bubble. Use sparingly: fine print that's really needed, never the main label. */
export const Tip: React.FC<{ content: React.ReactNode; children: React.ReactNode; className?: string; side?: 'top' | 'bottom' }> = ({ content, children, className, side = 'top' }) => {
  const ref = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const timer = useRef(0);
  const show = () => {
    timer.current = window.setTimeout(() => {
      const r = ref.current?.getBoundingClientRect();
      if (r) setPos({ x: r.left + r.width / 2, y: side === 'top' ? r.top : r.bottom });
    }, 280);
  };
  const hide = () => { clearTimeout(timer.current); setPos(null); };
  if (!content) return <span className={className}>{children}</span>;
  return (
    <span ref={ref} className={className} onMouseEnter={show} onMouseLeave={hide} onMouseDown={hide}>
      {children}
      {pos && createPortal(
        <div
          className="fixed z-[200] pointer-events-none glass-strong rounded-xl px-3 py-2 text-[13px] leading-snug max-w-[280px]"
          style={{ left: Math.min(window.innerWidth - 150, Math.max(150, pos.x)), top: pos.y + (side === 'top' ? -10 : 10), transform: `translate(-50%, ${side === 'top' ? '-100%' : '0'})` }}
        >
          {content}
        </div>,
        document.body,
      )}
    </span>
  );
};
