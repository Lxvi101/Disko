import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { AlertCircle, Info, Sparkles, Trash2, Undo2 } from 'lucide-react';
import { useStore } from '../../store';

const ICON = { freed: [Sparkles, 'var(--ok)'], trash: [Trash2, 'var(--muted)'], info: [Info, 'var(--accent-text)'], error: [AlertCircle, 'var(--danger)'] } as const;

export const Toasts: React.FC = () => {
  const toasts = useStore((s) => s.toasts);
  const undo = useStore((s) => s.undo);
  const dismiss = useStore((s) => s.dismiss);
  const hasSelection = useStore((s) => s.selection.size > 0);
  return (
    <div className="fixed right-6 z-[60] flex flex-col items-end gap-2.5 pointer-events-none transition-all" style={{ bottom: hasSelection ? 100 : 24 }}>
      <AnimatePresence initial={false}>
        {toasts.map((t) => {
          const [I, color] = ICON[t.kind];
          return (
            <motion.div
              key={t.id}
              layout
              initial={{ opacity: 0, y: 20, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, x: 40 }}
              transition={{ type: 'spring', stiffness: 500, damping: 36 }}
              className="pointer-events-auto glass-strong rounded-2xl flex items-center gap-3 pl-4 pr-2 py-2.5 min-w-[260px] max-w-[420px]"
              onClick={() => !t.undo && dismiss(t.id)}
            >
              <I size={20} style={{ color }} />
              <div className="flex-1 min-w-0">
                <div className="font-semibold text-[15px] num">{t.title}</div>
                {t.sub && <div className="muted text-[13px] truncate">{t.sub}</div>}
              </div>
              {t.undo && (
                <button className="btn !h-9" onClick={() => undo(t.id, t.undo!)}>
                  <Undo2 size={16} /> Undo
                </button>
              )}
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
};
