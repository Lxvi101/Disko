import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/** When a tip closed a moment ago, the next one opens without the delay (like macOS). */
let lastHidden = 0;
/** Only one tip at a time. */
let closeOpen: (() => void) | null = null;

/**
 * A soft hover bubble. Use sparingly: fine print that's really needed, never the main label.
 * Works over disabled buttons too (their pointer events go to this wrapper, see `.tip-host`).
 */
export const Tip: React.FC<{ content: React.ReactNode; children: React.ReactNode; className?: string; side?: 'top' | 'bottom' }> = ({ content, children, className, side = 'top' }) => {
  const ref = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number; below: boolean } | null>(null);
  const timer = useRef(0);

  const hide = () => {
    clearTimeout(timer.current);
    setPos((p) => { if (p) lastHidden = performance.now(); return null; });
    if (closeOpen === hide) closeOpen = null;
  };
  const place = () => {
    const el = ref.current;
    if (!el?.isConnected) return;
    // The wrapper may be `display: contents`-like inline; measure what's actually drawn.
    const r = el.getBoundingClientRect();
    if (!r.width && !r.height) return;
    const below = side === 'bottom' || r.top < 70;
    closeOpen?.();
    closeOpen = hide;
    setPos({ x: r.left + r.width / 2, y: below ? r.bottom : r.top, below });
  };
  const show = () => {
    clearTimeout(timer.current);
    const quick = performance.now() - lastHidden < 600;
    timer.current = window.setTimeout(place, quick ? 0 : 320);
  };

  useEffect(() => {
    if (!pos) return;
    const off = () => hide();
    window.addEventListener('scroll', off, true);
    window.addEventListener('wheel', off, { passive: true });
    window.addEventListener('blur', off);
    return () => {
      window.removeEventListener('scroll', off, true);
      window.removeEventListener('wheel', off);
      window.removeEventListener('blur', off);
    };
  }, [pos]);
  useEffect(() => () => { clearTimeout(timer.current); if (closeOpen === hide) closeOpen = null; }, []);

  if (!content) return <span className={className}>{children}</span>;
  return (
    <span
      ref={ref}
      className={`tip-host ${className ?? ''}`}
      onPointerEnter={show}
      onPointerLeave={hide}
      onPointerDown={hide}
    >
      {children}
      {pos && createPortal(
        <div
          role="tooltip"
          className="fixed z-[200] pointer-events-none glass-strong rounded-xl px-3 py-2 text-[13px] leading-snug max-w-[280px]"
          style={{
            left: Math.min(window.innerWidth - 150, Math.max(150, pos.x)),
            top: pos.y + (pos.below ? 10 : -10),
            transform: `translate(-50%, ${pos.below ? '0' : '-100%'})`,
          }}
        >
          {content}
        </div>,
        document.body,
      )}
    </span>
  );
};
