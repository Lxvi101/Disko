import React, { useEffect, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronLeft, ChevronRight, Search, Sparkles, RefreshCw, X, Sun, Moon, FolderSearch } from 'lucide-react';
import { useStore } from './store';
import { Assistant } from './components/Assistant';
import { Collector, CleanModal } from './components/Collector';
import { CommandPalette } from './components/CommandPalette';
import { Toasts } from './components/Toasts';
import { ContextMenuHost } from './components/ContextMenu';
import { ExplorePage } from './components/pages/Explore';
import { SuggestionsPage } from './components/pages/Suggestions';
import { InactivePage } from './components/pages/Inactive';
import { QuarantinePage } from './components/pages/Quarantine';
import { AppCleanupPage } from './components/pages/AppCleanup';
import { FilesPage } from './components/pages/Files';
import { ScanScreen } from './components/ScanScreen';
import { useScan } from './hooks/useScan';
import { Switch } from './components/ui';
import { formatBytes, formatNumber, relativeTime } from './lib/format';
import type { Page } from './types';

const PAGES: Record<Page, React.FC> = {
  apps: AppCleanupPage,
  explore: ExplorePage,
  files: FilesPage,
  suggestions: SuggestionsPage,
  inactive: InactivePage,
  quarantine: QuarantinePage,
};

const TABS: { id: Page; label: string }[] = [
  { id: 'explore', label: 'Map' },
  { id: 'files', label: 'Files' },
  { id: 'suggestions', label: 'Suggestions' },
  { id: 'inactive', label: 'Inactive' },
  { id: 'apps', label: 'App cleanup' },
  { id: 'quarantine', label: 'Quarantine' },
];

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
  const { booted, bootError, boot, page, setPage, assistantOpen, setAssistantOpen, paletteOpen, setPaletteOpen, currentPath, rootPath, navigate, navigateUp, goBack, goForward, canBack, canForward, advanced, setAdvanced, colorBy, setColorBy, theme, resolvedTheme, setTheme, applyTheme, startScreen, setStartScreen, info, overview, candidates, journals, cleanModalOpen, setCleanModalOpen, trayOpen, setTrayOpen, select } = s;
  const { scan, scanning, cancelScan } = useScan();

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
      if (meta && e.key === '[') { e.preventDefault(); goBack(); return; }
      if (meta && e.key === ']') { e.preventDefault(); goForward(); return; }
      if (e.key === 'Escape') {
        if (startScreen && info?.db_loaded && !scanning) setStartScreen(false);
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

  const vol = overview?.volume ?? info?.volume;
  const showScan = startScreen || scanning || (!!scan && scan.done && !scan.ok) || (!info?.db_loaded && page !== 'apps');
  const PageC = PAGES[page] ?? ExplorePage;
  const heldJournals = journals.filter((j) => j.items.some((i) => i.present)).length;

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

  return (
    <div className={`h-full w-full flex overflow-hidden ${advanced ? '' : 'standard-view'}`}>
      <div className="flex-1 min-w-0 flex flex-col relative">
        {/* top bar */}
        <header className="app-toolbar h-[52px] flex-shrink-0 flex items-center gap-2 pl-[84px] pr-4 drag-region">
          <div className="no-drag flex items-center gap-0.5">
            <button className="btn btn-ghost btn-icon" onClick={goBack} disabled={!canBack()} title="Back (⌘[)"><ChevronLeft className="w-4 h-4" /></button>
            <button className="btn btn-ghost btn-icon" onClick={goForward} disabled={!canForward()} title="Forward (⌘])"><ChevronRight className="w-4 h-4" /></button>
          </div>
          <nav aria-label="Folder path" className="breadcrumbs no-drag flex items-center gap-1 min-w-0 overflow-x-auto" style={{ scrollbarWidth: 'none' }}>
            {!advanced && <button className="breadcrumb" onClick={() => setStartScreen(true)}>Disks and Folders</button>}
            {crumbs.map((c, i) => {
              const last = i === crumbs.length - 1;
              return (
                <React.Fragment key={c.path}>
                  {(!advanced || i > 0) && <ChevronRight aria-hidden="true" className="breadcrumb-separator" />}
                  <button aria-current={last ? 'location' : undefined} onClick={() => { if (!last || page !== 'explore') navigate(c.path); }} className="breadcrumb">
                    {c.label}
                  </button>
                </React.Fragment>
              );
            })}
          </nav>

          <div className="flex-1" />

          <AnimatePresence initial={false}>
            {advanced && (
              <motion.nav key="tabs" initial={{ opacity: 0, x: 8 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: 8 }} className="no-drag flex items-center gap-0.5 mr-2">
                {TABS.map((t) => {
                  const active = page === t.id;
                  const count = t.id === 'suggestions' ? candidates.length : t.id === 'quarantine' ? heldJournals : 0;
                  return (
                    <button key={t.id} onClick={() => { setPage(t.id); if (t.id === 'apps') setStartScreen(false); }} className="relative px-2.5 h-7 rounded-md text-[12.5px] transition-colors" style={{ color: active ? 'var(--text)' : 'var(--muted)' }}>
                      {active && <motion.span layoutId="tab-bg" className="absolute inset-0 rounded-md" style={{ background: 'var(--bg-3)' }} transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
                      <span className="relative">{t.label}{count ? <span className="tnum ml-1" style={{ color: 'var(--dim)' }}>{count}</span> : null}</span>
                    </button>
                  );
                })}
              </motion.nav>
            )}
          </AnimatePresence>

          <div className="toolbar-actions no-drag flex items-center gap-1">
            <button className="btn btn-ghost btn-icon" onClick={() => setTheme(resolvedTheme === 'light' ? 'dark' : 'light')} title={theme === 'system' ? 'Appearance: follows system' : `Appearance: ${theme}`}>
              {resolvedTheme === 'light' ? <Moon className="w-4 h-4" /> : <Sun className="w-4 h-4" />}
            </button>
            <button className="btn btn-ghost btn-icon" onClick={() => setStartScreen(!startScreen)} title="Scans" style={{ color: showScan ? 'var(--text)' : undefined }}><FolderSearch className="w-4 h-4" /></button>
            <button className="btn btn-ghost btn-icon" onClick={() => setPaletteOpen(true)} title="Search (⌘K)"><Search className="w-4 h-4" /></button>
            <button className="btn btn-ghost btn-icon" onClick={() => setAssistantOpen(!assistantOpen)} title="Assistant (⌘J)" style={{ color: assistantOpen ? 'var(--text)' : undefined }}><Sparkles className="w-4 h-4" /></button>
            <div className="w-px h-4 mx-1" style={{ background: 'var(--line-2)' }} />
            <Switch on={advanced} onChange={setAdvanced} label="Advanced" />
          </div>
        </header>

        {/* advanced strip */}
        <AnimatePresence initial={false}>
          {advanced && (
            <motion.div key="strip" initial={{ height: 0, opacity: 0 }} animate={{ height: 32, opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="flex-shrink-0 overflow-hidden flex items-center gap-4 px-6 text-[11.5px]" style={{ color: 'var(--muted)' }}>
              {page === 'explore' && (
                <span className="flex items-center gap-1.5">
                  Colour by
                  {(['folder', 'policy'] as const).map((c) => (
                    <button key={c} onClick={() => setColorBy(c)} className="px-1.5 rounded transition-colors" style={{ color: colorBy === c ? 'var(--text)' : 'var(--dim)', background: colorBy === c ? 'var(--bg-3)' : 'transparent' }}>{c}</button>
                  ))}
                </span>
              )}
              {vol && <span className="tnum">{formatBytes(vol.available)} free of {formatBytes(vol.total)}</span>}
              {info?.scan?.entries ? <span className="tnum">{formatNumber(info.scan.entries)} entries · scanned {relativeTime(info.scan.finished ?? null)}</span> : <span>Not scanned yet</span>}
              {s.coverageIssues > 0 && <span className="tnum">{formatNumber(s.coverageIssues)} unreadable paths</span>}
              <span className="flex-1" />
              {scan && !scan.done ? (
                <span className="flex items-center gap-1.5"><RefreshCw className="w-3 h-3 animate-spin" />{scan.entries ? `${formatNumber(scan.entries)} entries` : scan.message}<button className="btn btn-ghost !p-0.5" onClick={cancelScan}><X className="w-3 h-3" /></button></span>
              ) : (
                <button className="flex items-center gap-1.5 transition-colors" style={{ color: 'var(--muted)' }} onClick={() => setStartScreen(true)} disabled={scanning}><RefreshCw className="w-3 h-3" /> New scan</button>
              )}
              {info?.codex_version && <span>{info.codex_version}</span>}
            </motion.div>
          )}
        </AnimatePresence>

        <main className="flex-1 min-h-0 relative">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div key={showScan ? 'scan' : page} className="absolute inset-0" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.12 }}>
              {showScan ? <ScanScreen /> : <PageC />}
            </motion.div>
          </AnimatePresence>
        </main>

        <Collector />
      </div>
      <ContextMenuHost />

      <AnimatePresence initial={false}>
        {assistantOpen && (
          <motion.div key="assistant" initial={{ width: 0, opacity: 0 }} animate={{ width: 380, opacity: 1 }} exit={{ width: 0, opacity: 0 }} transition={{ type: 'spring', stiffness: 340, damping: 36 }} className="h-full flex-shrink-0 overflow-hidden">
            <div className="w-[380px] h-full"><Assistant /></div>
          </motion.div>
        )}
      </AnimatePresence>
      <CleanModal />
      <CommandPalette />
      <Toasts />
    </div>
  );
};
