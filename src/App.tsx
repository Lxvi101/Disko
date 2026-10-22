import React, { useEffect, useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronLeft, ChevronRight, Search, Bot, Sun, Moon, PieChart, LayoutDashboard, CalendarClock, RefreshCw } from 'lucide-react';
import { useStore } from './store';
import { Assistant } from './components/Assistant';
import { Collector, CleanModal } from './components/Collector';
import { CommandPalette } from './components/CommandPalette';
import { Toasts } from './components/Toasts';
import { UpdatePrompt } from './components/UpdatePrompt';
import { ContextMenuHost } from './components/ContextMenu';
import { ExplorePage } from './components/pages/Explore';
import { SuggestionsPage } from './components/pages/Suggestions';
import { TimelinePage } from './components/pages/Timeline';
import { QuarantinePage } from './components/pages/Quarantine';
import { AppCleanupPage } from './components/pages/AppCleanup';
import { JunkPage } from './components/pages/Junk';
import { QuickCleanPage } from './components/pages/QuickClean';
import { Sidebar, RAIL, RAIL_OPEN } from './components/Sidebar';
import { ScanScreen } from './components/ScanScreen';
import { useScan } from './hooks/useScan';
import { Switch, SegmentedControl } from './components/ui';
import { formatNumber, relativeTime } from './lib/format';
import type { Page } from './types';

const PAGES: Record<Page, React.FC> = {
  quick: QuickCleanPage,
  apps: AppCleanupPage,
  explore: ExplorePage,
  junk: JunkPage,
  suggestions: SuggestionsPage,
  timeline: TimelinePage,
  quarantine: QuarantinePage,
};

/** Pages that read the disk scan; without one they show the scan screen. */
const NEEDS_SCAN: Page[] = ['explore', 'junk', 'suggestions', 'timeline'];

export const App: React.FC = () => {
  useEffect(() => {
    // The webview's own right-click menu (Reload, etc.) is never useful here; text fields keep theirs.
    const h = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest('input, textarea, [contenteditable="true"], .selectable')) return;
      e.preventDefault();
    };
    window.addEventListener('contextmenu', h);
    return () => window.removeEventListener('contextmenu', h);
  }, []);
  const s = useStore();
  const { booted, bootError, boot, page, assistantOpen, setAssistantOpen, paletteOpen, setPaletteOpen, currentPath, rootPath, navigate, navigateUp, goBack, goForward, canBack, canForward, advanced, setAdvanced, colorBy, setColorBy, theme, resolvedTheme, setTheme, applyTheme, startScreen, setStartScreen, info, cleanModalOpen, setCleanModalOpen, trayOpen, setTrayOpen, select } = s;
  const { scan, scanning } = useScan();
  const [pinned, setPinnedState] = useState(() => { try { return localStorage.getItem('disko.pinned') === '1'; } catch { return false; } });
  const setPinned = (v: boolean) => { setPinnedState(v); try { localStorage.setItem('disko.pinned', v ? '1' : '0'); } catch {} };

  useEffect(() => { boot(); }, [boot]);

  // Follow the system appearance while theme is 'system'.
  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-color-scheme: light)');
    if (!mq) return;
    const h = () => applyTheme();
    mq.addEventListener('change', h);
    return () => mq.removeEventListener('change', h);
  }, [applyTheme]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      const tag = (e.target as HTMLElement)?.tagName;
      const typing = tag === 'INPUT' || tag === 'TEXTAREA';
      if (meta && e.key.toLowerCase() === 'k') { e.preventDefault(); setPaletteOpen(!paletteOpen); return; }
      if (meta && e.key.toLowerCase() === 'j') { e.preventDefault(); setAssistantOpen(!assistantOpen); return; }
      if (meta && e.key === '.') { e.preventDefault(); setAdvanced(!advanced); return; }
      if (meta && e.key === '\\') { e.preventDefault(); setPinned(!pinnedRef.current); return; }
      if (meta && e.key === '[') { e.preventDefault(); goBack(); return; }
      if (meta && e.key === ']') { e.preventDefault(); goForward(); return; }
      if (e.key === 'Escape') {
        if (startScreen && !scanning) setStartScreen(false);
        else if (cleanModalOpen) setCleanModalOpen(false);
        else if (paletteOpen) setPaletteOpen(false);
        else if (trayOpen) setTrayOpen(false);
        else select(null);
        return;
      }
      if (!typing && (e.key === 'Backspace' || (meta && e.key === 'ArrowUp')) && page === 'explore') { e.preventDefault(); navigateUp(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [paletteOpen, assistantOpen, cleanModalOpen, trayOpen, page, advanced, navigateUp, select, setAssistantOpen, setCleanModalOpen, setPaletteOpen, setTrayOpen, setAdvanced, goBack, goForward, startScreen, setStartScreen, info?.db_loaded, scanning]);

  const pinnedRef = React.useRef(pinned);
  pinnedRef.current = pinned;

  const crumbs = useMemo(() => {
    if (!currentPath) return [] as { label: string; path: string }[];
    const rel = currentPath.startsWith(rootPath) ? currentPath.slice(rootPath.length) : currentPath;
    const parts = rel.split('/').filter(Boolean);
    const rootLabel = rootPath === '/' ? 'Macintosh HD' : rootPath === info?.home ? 'Home' : rootPath.split('/').filter(Boolean).pop() ?? 'Root';
    const out = [{ label: rootLabel, path: rootPath }];
    let acc = rootPath;
    for (const p of parts) { acc = `${acc.replace(/\/$/, '')}/${p}`; out.push({ label: p, path: acc }); }
    return out;
  }, [currentPath, rootPath, info?.home]);

  const showScan = startScreen || scanning || (!!scan && scan.done && !scan.ok) || (!info?.db_loaded && NEEDS_SCAN.includes(page));
  const PageC = PAGES[page] ?? ExplorePage;

  if (!booted) {
    return (
      <div className="h-full w-full flex items-center justify-center">
        <motion.div className="w-10 h-10 rounded-full" style={{ border: '1.5px solid var(--line-2)', borderTopColor: 'var(--text)' }} animate={{ rotate: 360 }} transition={{ repeat: Infinity, duration: 0.9, ease: 'linear' }} />
      </div>
    );
  }
  if (bootError) {
    return (
      <div className="h-full w-full flex items-center justify-center p-8">
        <div className="max-w-md text-center">
          <h2 className="text-[14px] font-semibold">Disko could not start</h2>
          <p className="text-[12.5px] mt-1.5 selectable" style={{ color: 'var(--muted)' }}>{bootError}</p>
          <button className="btn mt-4" onClick={boot}>Retry</button>
        </div>
      </div>
    );
  }

  const onMap = page === 'explore' && !showScan;
  return (
    <div className="app-shell h-full w-full flex overflow-hidden">
      <Sidebar onScan={() => setStartScreen(!startScreen)} scanning={scanning} pinned={pinned} onPin={setPinned} />
      <motion.div className="app-main flex-1 min-w-0 flex flex-col" initial={false} animate={{ paddingLeft: pinned ? RAIL_OPEN : RAIL }} transition={{ type: 'spring', stiffness: 420, damping: 38 }}>
        {/* top bar */}
        <header className="app-toolbar h-[52px] flex-shrink-0 flex items-center gap-2 px-4 drag-region">
          {onMap && (
            <>
              <div className="no-drag flex items-center gap-0.5">
                <button className="btn btn-ghost btn-icon" onClick={goBack} disabled={!canBack()} title="Back (⌘[)"><ChevronLeft className="w-4 h-4" /></button>
                <button className="btn btn-ghost btn-icon" onClick={goForward} disabled={!canForward()} title="Forward (⌘])"><ChevronRight className="w-4 h-4" /></button>
              </div>
              <nav aria-label="Folder path" className="breadcrumbs no-drag flex items-center gap-1 min-w-0 overflow-x-auto" style={{ scrollbarWidth: 'none' }}>
                {crumbs.map((c, i) => {
                  const last = i === crumbs.length - 1;
                  return (
                    <React.Fragment key={c.path}>
                      {i > 0 && <ChevronRight aria-hidden="true" className="breadcrumb-separator" />}
                      <button aria-current={last ? 'location' : undefined} onClick={() => { if (!last) navigate(c.path); }} className="breadcrumb">
                        {c.label}
                      </button>
                    </React.Fragment>
                  );
                })}
              </nav>
            </>
          )}

          <div className="flex-1" />

          <div className="toolbar-actions no-drag flex items-center gap-1">
            {onMap && advanced && (
              <span className="flex items-center gap-1 text-[11.5px] mr-2" style={{ color: 'var(--muted)' }}>
                Colour by
                {(['folder', 'policy'] as const).map((c) => (
                  <button key={c} onClick={() => setColorBy(c)} className="px-1.5 rounded transition-colors" style={{ color: colorBy === c ? 'var(--text)' : 'var(--dim)', background: colorBy === c ? 'var(--bg-3)' : 'transparent' }}>{c}</button>
                ))}
              </span>
            )}
            {onMap && s.coverageIssues > 0 && <span className="tnum text-[11.5px] mr-2" style={{ color: 'var(--dim)' }} title="Folders macOS did not let Disko read. Grant Full Disk Access and rescan to include them.">{formatNumber(s.coverageIssues)} unreadable</span>}
            {onMap && info?.scan?.finished && (
              // The map is a snapshot; say how old it is and make rescanning one click away.
              <button className="btn btn-ghost btn-sm mr-1" onClick={() => setStartScreen(true)} title="The map shows your last scan. Scan again to see what changed since."
                style={{ color: Date.now() / 1000 - info.scan.finished > 3 * 86400 ? 'var(--warn)' : 'var(--muted)' }}>
                <RefreshCw className="w-3 h-3" /> Scanned {relativeTime(info.scan.finished)}
              </button>
            )}
            {onMap && (
              <span className="mr-2">
                <SegmentedControl
                  value={s.viz === 'treemap' || s.viz === 'timeline' ? s.viz : 'sunburst'}
                  onChange={s.setViz}
                  options={[
                    { value: 'sunburst', label: <PieChart className="w-3.5 h-3.5" />, title: 'Ring view' },
                    { value: 'treemap', label: <LayoutDashboard className="w-3.5 h-3.5" />, title: 'Block view' },
                    { value: 'timeline', label: <CalendarClock className="w-3.5 h-3.5" />, title: 'Timeline: when this folder last changed' },
                  ]}
                />
              </span>
            )}
            {onMap && <span className="mr-1"><Switch on={advanced} onChange={setAdvanced} label="Details" /></span>}
            <button className="btn btn-ghost btn-icon" onClick={() => setTheme(resolvedTheme === 'light' ? 'dark' : 'light')} title={theme === 'system' ? 'Appearance: follows system' : `Appearance: ${theme}`}>
              {resolvedTheme === 'light' ? <Moon className="w-4 h-4" /> : <Sun className="w-4 h-4" />}
            </button>
            <button className="btn btn-ghost btn-icon" onClick={() => setPaletteOpen(true)} title="Search (⌘K)"><Search className="w-4 h-4" /></button>
            <button className="btn btn-ghost btn-icon" onClick={() => setAssistantOpen(!assistantOpen)} title={`Assistant (⌘J)${info?.codex_version ? ` · ${info.codex_version}` : ''}`} style={{ color: assistantOpen ? 'var(--text)' : undefined }}><Bot className="w-4 h-4" /></button>
          </div>
        </header>

        <main className="flex-1 min-h-0 relative">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div key={showScan ? 'scan' : page} className="absolute inset-0" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.12 }}>
              {showScan ? <ScanScreen /> : <PageC />}
            </motion.div>
          </AnimatePresence>
        </main>

        {page !== 'quick' && <Collector />}
      </motion.div>
      <ContextMenuHost />

      <AnimatePresence initial={false}>
        {assistantOpen && (
          <motion.div key="assistant" initial={{ width: 0, opacity: 0 }} animate={{ width: 400, opacity: 1 }} exit={{ width: 0, opacity: 0 }} transition={{ type: 'spring', stiffness: 340, damping: 36 }} className="app-main h-full flex-shrink-0 overflow-hidden">
            <div className="w-[400px] h-full"><Assistant /></div>
          </motion.div>
        )}
      </AnimatePresence>
      <CleanModal />
      <CommandPalette />
      <Toasts />
      <UpdatePrompt />
    </div>
  );
};
