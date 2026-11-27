import React, { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CheckCircle2, Shield, Trash2, type LucideIcon } from 'lucide-react';
import { useStore, type SelItem } from '../../store';
import { splitBytes } from '../../lib/format';
import { dragged, endDrag } from '../../lib/interact';

/** Appears while something is dragged: drop it to select, keep or remove it. */
export const DropDock: React.FC = () => {
  const dragging = useStore((s) => s.dragging);
  const items = dragging ?? [];
  const permanent = items.some((i) => i.permanent);
  const allTrash = items.every((i) => i.trash);
  const t = splitBytes(items.reduce((a, i) => a + i.bytes, 0));
  const act = (fn: (items: SelItem[]) => void) => () => {
    const its = dragged();
    endDrag();
    if (its.length) fn(its);
  };
  const st = useStore.getState;
  return (
    <AnimatePresence>
      {dragging && (
        <motion.div
          initial={{ y: 110, opacity: 0, x: '-50%' }}
          animate={{ y: 0, opacity: 1, x: '-50%' }}
          exit={{ y: 110, opacity: 0, x: '-50%' }}
          transition={{ type: 'spring', stiffness: 460, damping: 34 }}
          className="fixed bottom-6 left-[calc(50%+38px)] z-[70] glass-strong rounded-[30px] flex items-center gap-2 p-2"
        >
          <div className="px-4 flex flex-col">
            <span className="text-[13px] muted truncate max-w-[180px]">{items.length === 1 ? items[0].title : `${items.length} items`}</span>
            <span className="flex items-baseline gap-1"><span className="num text-[22px] font-semibold leading-tight">{t.value}</span><span className="num unit text-[14px]">{t.unit}</span></span>
          </div>
          <Target icon={CheckCircle2} label="Select" onDrop={act((its) => st().setMany(its, true))} />
          <Target icon={Shield} label="Always keep" onDrop={act((its) => st().keep(its))} />
          <Target
            icon={Trash2}
            label={allTrash ? 'Move to Trash' : 'Remove'}
            danger
            disabled={permanent ? 'Emptying the Trash needs press and hold' : undefined}
            onDrop={act((its) => st().remove(its))}
          />
        </motion.div>
      )}
    </AnimatePresence>
  );
};

const Target: React.FC<{ icon: LucideIcon; label: string; onDrop: () => void; danger?: boolean; disabled?: string }> = ({ icon: I, label, onDrop, danger, disabled }) => {
  const [over, setOver] = useState(false);
  const tone = danger ? 'var(--danger)' : 'var(--accent-text)';
  return (
    <div
      onDragEnter={(e) => { if (!disabled) { e.preventDefault(); setOver(true); } }}
      onDragOver={(e) => { if (!disabled) { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; } }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => { e.preventDefault(); setOver(false); if (!disabled) onDrop(); }}
      title={disabled}
      className="relative flex flex-col items-center justify-center gap-1.5 w-[118px] h-[84px] rounded-[24px] transition-all"
      style={{
        opacity: disabled ? 0.4 : 1,
        background: over ? `color-mix(in srgb, ${tone} 22%, transparent)` : 'var(--panel)',
        boxShadow: `inset 0 0 0 ${over ? 2 : 1}px ${over ? tone : 'var(--line)'}`,
        transform: over ? 'scale(1.06)' : undefined,
        color: over ? tone : 'var(--text)',
      }}
    >
      <I size={24} strokeWidth={1.7} />
      <span className="text-[13px] font-medium">{label}</span>
    </div>
  );
};
