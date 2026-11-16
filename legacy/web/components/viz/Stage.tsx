import React, { useState } from 'react';
import { motion, AnimatePresence, type Variants } from 'framer-motion';
import { isInside } from '../../lib/sunburst';

export type Direction = 'in' | 'out' | 'none';

export function navDirection(from: string, to: string): Direction {
  return isInside(to, from) ? 'in' : isInside(from, to) ? 'out' : 'none';
}

/**
 * Keep drawing the last finished layout while the next folder loads, so a navigation
 * is one swap instead of old → empty → new, and remember which way it went.
 */
export function useHeldLayout<L>(layout: L | null, path: string | null): { layout: L; path: string; direction: Direction } | null {
  const [held, setHeld] = useState<{ layout: L; path: string; direction: Direction } | null>(null);
  if (layout && path && held?.layout !== layout) {
    const direction = !held || held.path === path ? held?.direction ?? 'none' : navDirection(held.path, path);
    setHeld({ layout, path, direction });
  }
  return held;
}

/** Positive curve: fast start, soft landing, no overshoot. */
export const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);
const EASE = [0.2, 0.8, 0.2, 1] as const;

// Zooming in: the old view grows past the viewer while the child grows into place.
// Zooming out is the exact reverse. Exit variants read `custom` at exit time, so the
// leaving view always uses the direction of the navigation that replaced it.
const variants: Variants = {
  enter: (d: Direction) => ({ scale: d === 'in' ? 0.6 : d === 'out' ? 1.4 : 0.97, opacity: 0, zIndex: 1 }),
  center: { scale: 1, opacity: 1, zIndex: 1, transition: { duration: 0.22, ease: EASE } },
  exit: (d: Direction) => ({ scale: d === 'in' ? 1.4 : d === 'out' ? 0.6 : 1, opacity: 0, zIndex: 0, transition: { duration: 0.16, ease: EASE } }),
};

export const ZoomStage: React.FC<{ id: string; direction: Direction; width: number; height: number; children: React.ReactNode }> = ({ id, direction, width, height, children }) => (
  <AnimatePresence initial={false} custom={direction}>
    <motion.div
      key={id}
      custom={direction}
      variants={variants}
      initial="enter"
      animate="center"
      exit="exit"
      className="absolute"
      style={{ left: '50%', top: '50%', marginLeft: -width / 2, marginTop: -height / 2, width, height, willChange: 'transform, opacity' }}
    >
      {children}
    </motion.div>
  </AnimatePresence>
);
