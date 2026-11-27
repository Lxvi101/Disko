import React, { useEffect } from 'react';
import { AnimatePresence, MotionConfig, motion } from 'framer-motion';
import { useStore } from './store';
import { Rail } from './components/shell/Rail';
import { TopBar } from './components/shell/TopBar';
import { SelectionBar } from './components/shell/SelectionBar';
import { Toasts } from './components/shell/Toasts';
import { UpdatePrompt } from './components/shell/UpdatePrompt';
import { DropDock } from './components/shell/DropDock';
import { ContextMenu } from './components/shell/ContextMenu';
import { api } from './lib/api';
import { Space } from './sections/space/Space';
import { Clutter } from './sections/Clutter';
import { Apps } from './sections/Apps';
import { Agents } from './sections/Agents';
import { Memory } from './sections/Memory';
import { Settings } from './sections/Settings';

const SECTIONS = { space: Space, clutter: Clutter, apps: Apps, agents: Agents, memory: Memory, settings: Settings };

export const App: React.FC = () => {
  const section = useStore((s) => s.section);
  const init = useStore((s) => s.init);
  useEffect(() => { init(); }, [init]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const st = useStore.getState();
      const typing = e.target instanceof HTMLElement && (e.target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName));
      if (e.key === 'Escape') { if (st.menu) st.closeMenu(); else st.clearSelection(); }
      // Space previews what the pointer is over, like Finder.
      if (e.key === ' ' && !typing && st.hover && st.hover.startsWith('/') && !st.hover.endsWith('/…')) {
        e.preventDefault();
        api.quicklook(st.hover);
      }
      if (e.metaKey && /^[1-5]$/.test(e.key)) {
        e.preventDefault();
        useStore.getState().go((['space', 'clutter', 'apps', 'agents', 'memory'] as const)[Number(e.key) - 1]);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const Current = SECTIONS[section];
  return (
    <MotionConfig reducedMotion="user">
      <div className="app-bg h-full w-full flex">
        <Rail />
        <main className="flex-1 min-w-0 flex flex-col">
          <TopBar />
          <div className="flex-1 min-h-0 relative">
            {/* Crossfade: the next section starts right away instead of waiting for the old one to leave. */}
            <AnimatePresence initial={false}>
              <motion.div key={section} className="absolute inset-0" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, pointerEvents: 'none', transition: { duration: 0.1 } }} transition={{ duration: 0.16 }}>
                <Current />
              </motion.div>
            </AnimatePresence>
          </div>
        </main>
        <SelectionBar />
        <DropDock />
        <ContextMenu />
        <Toasts />
        <UpdatePrompt />
      </div>
    </MotionConfig>
  );
};
