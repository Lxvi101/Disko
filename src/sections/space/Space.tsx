import React, { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ChartPie, ChevronLeft, ChevronRight, HardDrive, Home, LayoutGrid, List, ShieldCheck, CircleDot } from 'lucide-react';
import { useStore, type SpaceView } from '../../store';
import { api } from '../../lib/api';
import { parentOf } from '../../lib/format';
import { Tip } from '../../components/ui/Tip';
import { GlassIcon } from '../../lib/icons';
import { useSpaceTree } from './model';
import { Sunburst } from './Sunburst';
import { Bubbles } from './Bubbles';
import { Blocks } from './Blocks';
import { FolderList, ListView } from './FolderList';

const VIEWS: { id: SpaceView; icon: typeof ChartPie; label: string }[] = [
  { id: 'sunburst', icon: ChartPie, label: 'Sunburst' },
  { id: 'bubbles', icon: CircleDot, label: 'Bubbles' },
  { id: 'blocks', icon: LayoutGrid, label: 'Blocks' },
  { id: 'list', icon: List, label: 'List' },
];

export const Space: React.FC = () => {
  const { tree } = useSpaceTree();
  const view = useStore((s) => s.spaceView);
  const boot = useStore((s) => s.boot);
  return (
    <div className="flex flex-col h-full min-h-0 px-5 pb-5 gap-4">
      {boot && !boot.full_disk_access && <AccessCard />}
      <div className="flex items-center gap-3">
        <Breadcrumb />
        <div className="flex-1" />
        <ViewSwitch />
      </div>
      <div className="flex-1 min-h-0 flex gap-4">
        {view !== 'list' && (
          <div className="glass rounded-[28px] w-[clamp(320px,30vw,420px)] shrink-0 min-h-0 overflow-hidden">
            {tree ? <FolderList tree={tree} /> : <Skeleton />}
          </div>
        )}
        <div className={`flex-1 min-w-0 min-h-0 relative ${view === 'list' ? 'glass rounded-[28px] pt-3' : view === 'sunburst' ? 'p-2' : 'p-1'}`}>
          {tree ? (
            view === 'sunburst' ? <Sunburst tree={tree} />
            : view === 'bubbles' ? <Bubbles tree={tree} />
            : view === 'blocks' ? <Blocks tree={tree} />
            : <ListView tree={tree} />
          ) : (
            <div className="w-full h-full flex items-center justify-center"><div className="w-[60%] aspect-square rounded-full shimmer" /></div>
          )}
        </div>
      </div>
    </div>
  );
};

const Skeleton = () => (
  <div className="p-5 flex flex-col gap-3">
    {Array.from({ length: 8 }, (_, i) => <div key={i} className="h-12 rounded-2xl shimmer" style={{ opacity: 1 - i * 0.1 }} />)}
  </div>
);

const Breadcrumb: React.FC = () => {
  const path = useStore((s) => s.spacePath);
  const openFolder = useStore((s) => s.openFolder);
  const home = useStore((s) => s.boot?.home);
  const parts = path === '/' ? [] : path.split('/').filter(Boolean);
  const crumbs = [{ name: 'Macintosh HD', path: '/' }, ...parts.map((p, i) => ({ name: p, path: '/' + parts.slice(0, i + 1).join('/') }))];
  const up = parentOf(path);
  return (
    <div className="no-drag flex items-center gap-1 min-w-0">
      <button className="btn btn-icon" aria-label="Back" disabled={!up} onClick={() => up && openFolder(up)}><ChevronLeft size={20} /></button>
      <div className="flex items-center min-w-0 overflow-hidden ml-1">
        {crumbs.map((c, i) => (
          <React.Fragment key={c.path}>
            {i > 0 && <ChevronRight size={16} className="dim shrink-0" />}
            <button
              onClick={() => openFolder(c.path)}
              className="flex items-center gap-1.5 px-2 py-1 rounded-lg row-hover shrink min-w-0"
              style={{ fontWeight: i === crumbs.length - 1 ? 600 : 500, fontSize: 15, color: i === crumbs.length - 1 ? 'var(--text)' : 'var(--muted)' }}
            >
              {i === 0 && <HardDrive size={17} />}
              {c.path === home && <Home size={16} />}
              <span className="truncate">{c.name}</span>
            </button>
          </React.Fragment>
        ))}
      </div>
      {path === '/' && home && (
        <button className="btn ml-2 !h-8 text-[13px]" onClick={() => openFolder(home)}>Home folder</button>
      )}
    </div>
  );
};

const ViewSwitch: React.FC = () => {
  const view = useStore((s) => s.spaceView);
  const setView = useStore((s) => s.setView);
  return (
    <div className="no-drag seg rounded-full p-1 flex gap-0.5">
      {VIEWS.map((v) => (
        <Tip key={v.id} content={v.label} side="bottom">
          <button aria-label={v.label} onClick={() => setView(v.id)} className="relative w-11 h-8 rounded-full flex items-center justify-center transition-colors" style={{ color: view === v.id ? 'var(--text)' : 'var(--muted)' }}>
            {view === v.id && <motion.span layoutId="view-pill" className="absolute inset-0 rounded-full seg-on" transition={{ type: 'spring', stiffness: 420, damping: 40 }} />}
            <v.icon size={18} strokeWidth={1.7} className="relative" />
          </button>
        </Tip>
      ))}
    </div>
  );
};

/** Inline, never blocking. */
const AccessCard: React.FC = () => {
  const [asked, setAsked] = useState(false);
  const refreshBoot = useStore((s) => s.refreshBoot);
  const hidden = useStore((s) => s.volume?.hidden.find((h) => h.kind === 'unreadable'));
  return (
    <AnimatePresence>
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }} className="no-drag glass rounded-[24px] flex items-center gap-4 px-4 py-3.5">
        <GlassIcon icon={ShieldCheck} color="#6e8dcf" size={44} />
        <div className="flex-1 min-w-0">
          <div className="text-[16px] font-semibold">Let Disko see everything</div>
          <div className="muted text-[13.5px]">{hidden ? `Some folders are private to macOS until you allow it.` : 'Mail, Messages and a few system folders stay hidden until you allow it.'}</div>
        </div>
        {asked ? (
          <button className="btn" onClick={() => refreshBoot()}>I turned it on</button>
        ) : (
          <button className="btn btn-primary" onClick={() => { setAsked(true); api.openFullDiskAccess(); }}>Open Settings</button>
        )}
      </motion.div>
    </AnimatePresence>
  );
};
