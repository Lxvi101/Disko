import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Trash2, X } from 'lucide-react';
import { useStore } from '../../store';
import { splitBytes } from '../../lib/format';
import { Icon, NodeIcon } from '../../lib/icons';
import { CAT } from '../../lib/categories';
import { Glyph } from '../../lib/icons';
import { HoldButton } from '../ui/HoldButton';

/** Floats over every section whenever something is selected. No confirmation: Undo lives in the toast. */
export const SelectionBar: React.FC = () => {
  const selection = useStore((s) => s.selection);
  const clear = useStore((s) => s.clearSelection);
  const remove = useStore((s) => s.remove);
  const items = [...selection.values()];
  const total = items.reduce((a, i) => a + i.bytes, 0);
  const t = splitBytes(total);
  const permanent = items.some((i) => i.permanent);
  const allSafe = items.every((i) => i.safety === 'safe');
  return (
    <AnimatePresence>
      {items.length > 0 && (
        <motion.div
          initial={{ y: 90, opacity: 0, x: '-50%' }}
          animate={{ y: 0, opacity: 1, x: '-50%' }}
          exit={{ y: 90, opacity: 0, x: '-50%' }}
          transition={{ type: 'spring', stiffness: 420, damping: 34 }}
          className="fixed bottom-6 left-[calc(50%+38px)] z-50 glass-strong rounded-full flex items-center gap-4 pl-3 pr-2.5 py-2.5"
        >
          <div className="flex -space-x-2.5">
            {items.slice(0, 4).map((i) => (
              <span key={i.key} className="rounded-[11px] ring-2" style={{ ['--tw-ring-color' as string]: 'var(--solid)' }}>
                {i.icon ? <Icon iconKey={i.icon} size={34} /> : i.category ? <Glyph name="x" icon={CAT[i.category].icon} color={CAT[i.category].color} size={34} /> : <NodeIcon node={{ name: i.title, path: i.paths[0], kind: 'dir' }} size={34} />}
              </span>
            ))}
          </div>
          <div className="flex items-baseline gap-2 pr-1">
            <span className="num text-[24px] font-semibold leading-none">{t.value}</span>
            <span className="num text-[15px] unit">{t.unit}</span>
            <span className="muted text-[15px] ml-1">· {items.length} {items.length === 1 ? 'item' : 'items'}</span>
          </div>
          <button className="btn btn-ghost btn-icon" aria-label="Clear selection" onClick={clear}><X size={18} /></button>
          {permanent ? (
            <HoldButton className="btn-lg" onDone={() => remove(items)}><Trash2 size={18} /> Hold to delete forever</HoldButton>
          ) : (
            <button className="btn btn-primary btn-lg" onClick={() => remove(items)}>
              <Trash2 size={18} /> {allSafe ? 'Remove' : 'Move to Trash'}
            </button>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
};
