import React from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { CheckCircle2, Info, AlertTriangle, X } from 'lucide-react';
import { useStore } from '../store';

export const Toasts: React.FC = () => {
  const { toasts, dismissToast } = useStore();
  return (
    <div className="fixed top-4 right-4 z-[100] flex flex-col gap-2 w-[340px] pointer-events-none">
      <AnimatePresence initial={false}>
        {toasts.map((t) => {
          const color = t.kind === 'success' ? '#34d399' : t.kind === 'error' ? '#fb7185' : '#8b7cf6';
          const Icon = t.kind === 'success' ? CheckCircle2 : t.kind === 'error' ? AlertTriangle : Info;
          return (
            <motion.div
              key={t.id}
              layout
              initial={{ opacity: 0, x: 40, scale: 0.96 }}
              animate={{ opacity: 1, x: 0, scale: 1 }}
              exit={{ opacity: 0, x: 40, scale: 0.96 }}
              transition={{ type: 'spring', stiffness: 400, damping: 30 }}
              className="glass-strong rounded-2xl p-3 flex items-start gap-3 pointer-events-auto"
            >
              <Icon className="w-4 h-4 mt-0.5 flex-shrink-0" style={{ color }} />
              <div className="flex-1 min-w-0">
                <div className="text-[12.5px] font-semibold">{t.title}</div>
                {t.detail && <div className="text-[11.5px] mt-0.5 leading-snug break-words" style={{ color: 'var(--muted)' }}>{t.detail}</div>}
              </div>
              <button className="btn btn-ghost !p-0.5 -mr-1 -mt-0.5" onClick={() => dismissToast(t.id)}>
                <X className="w-3.5 h-3.5" />
              </button>
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
};
