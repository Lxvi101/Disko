import React, { useEffect, useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Home, HardDrive, FolderSearch, Shield, ChevronRight, X, Lock } from 'lucide-react';
import { useStore } from '../store';
import { useScan } from '../hooks/useScan';
import { api, errorText } from '../lib/api';
import { formatBytes, formatNumber, relativeTime, formatDuration, shortPath } from '../lib/format';
import type { ScanInfo } from '../types';

type Target = 'home' | 'disk' | 'folder';

export const ScanScreen: React.FC = () => {
  const { info, scans, refreshScans, setStartScreen, boot, reloadData, toast, autoscan } = useStore();
  const { scan, scanning, startScan, cancelScan } = useScan();
  const [target, setTarget] = useState<Target>('home');
  const [quick, setQuick] = useState(true);
  const [admin, setAdmin] = useState(false);
  const [folder, setFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [fda, setFda] = useState<boolean | null>(null);
  const [fdaDismissed, setFdaDismissed] = useState(false);
  const [fdaBusy, setFdaBusy] = useState(false);
  const [fdaRequested, setFdaRequested] = useState(false);
  const [fdaMessage, setFdaMessage] = useState<string | null>(null);

  useEffect(() => {
    refreshScans();
    api.fullDiskAccess().then(setFda).catch(() => setFda(null));
  }, [refreshScans]);

  // Re-check when the window regains focus (after visiting System Settings).
  useEffect(() => {
    const h = () => api.fullDiskAccess().then(setFda).catch(() => {});
    window.addEventListener('focus', h);
    return () => window.removeEventListener('focus', h);
  }, []);

  // Launch-time automation: DISKO_AUTOSCAN=quick|thorough
  useEffect(() => {
    if (!autoscan || !info?.home) return;
    useStore.setState({ autoscan: null });
    startScan(info.home, { quick: autoscan !== 'thorough' });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoscan, info?.home]);

  const active = useMemo(() => scans.find((s) => s.active) ?? null, [scans]);

  const requestAccess = async () => {
    setFdaBusy(true);
    setFdaMessage(null);
    try {
      const granted = await api.requestFullDiskAccess();
      setFda(granted);
      setFdaRequested(true);
      if (!granted) setFdaMessage('Turn on Disko in Full Disk Access. If it is missing, click + and choose Disko.app, or drag it from Finder into the list. Then quit and reopen Disko.');
    } catch (e) {
      setFdaMessage(errorText(e));
    } finally {
      setFdaBusy(false);
    }
  };

  const checkAccess = async () => {
    setFdaBusy(true);
    setFdaMessage(null);
    try {
      const granted = await api.fullDiskAccess();
      setFda(granted);
      if (!granted) setFdaMessage('Access is still unavailable. Request access again, or quit and reopen Disko if you already enabled it in Settings.');
    } catch (e) {
      setFdaMessage(errorText(e));
    } finally {
      setFdaBusy(false);
    }
  };


  const pickFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const dir = await open({ directory: true, multiple: false, title: 'Choose a folder to scan' });
      if (typeof dir === 'string' && dir) {
        setFolder(dir);
        setTarget('folder');
      }
    } catch (e) {
      toast({ kind: 'error', title: 'Could not open folder picker', detail: errorText(e) });
    }
  };

  const begin = () => {
    const root = target === 'home' ? info?.home : target === 'disk' ? '/' : folder;
    if (!root) return pickFolder();
    startScan(root, { quick, admin: admin && target === 'disk' });
  };

  const openScan = async (s: ScanInfo) => {
    setBusy(s.path);
    try {
      await api.setActiveScan(s.path);
      await boot();
      await reloadData();
      setStartScreen(false);
    } catch (e) {
      toast({ kind: 'error', title: 'Could not open scan', detail: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  if (scanning || (scan && scan.done && !scan.ok)) {
    return <ProgressView onCancel={cancelScan} />;
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[720px] mx-auto px-8 pt-14 pb-16">
        <h1 className="text-[22px] font-semibold tracking-tight">What would you like to look at?</h1>
        <p className="text-[13px] mt-1.5" style={{ color: 'var(--muted)' }}>
          A scan reads sizes and dates only. Your files are never changed. Only the latest completed scan is kept.
        </p>

        {fda === false && !fdaDismissed && (
          <div className="mt-6 rounded-xl p-4 flex items-start gap-3" style={{ border: '1px solid var(--line-2)' }}>
            <Lock className="w-4 h-4 mt-0.5 flex-shrink-0" style={{ color: 'var(--muted)' }} />
            <div className="flex-1 min-w-0 text-[12.5px]">
              <div className="font-medium">Grant Full Disk Access once</div>
              <p className="mt-1 leading-relaxed" style={{ color: 'var(--muted)' }}>
                Allow Disko to measure protected folders. Request access, then enable Disko in System Settings → Privacy &amp; Security → Full Disk Access. Protected folders are skipped until access is granted.
              </p>
              <div className="mt-3 flex flex-wrap items-center gap-2">
                <button className="btn btn-primary btn-sm" disabled={fdaBusy} onClick={requestAccess}>{fdaBusy ? 'Checking access…' : fdaRequested ? 'Request access again' : 'Request access'}</button>
                <button className="btn btn-ghost btn-sm" disabled={fdaBusy} onClick={checkAccess}>Check again</button>
                <button className="btn btn-ghost btn-sm" disabled={fdaBusy} onClick={() => setFdaDismissed(true)}>Not now</button>
              </div>
              {fdaMessage && <p role="status" className="mt-3 leading-relaxed" style={{ color: 'var(--muted)' }}>{fdaMessage}</p>}
              <button className="btn btn-ghost btn-sm mt-2" onClick={() => api.revealRunningApp().catch(e => setFdaMessage(errorText(e)))}>Show Disko in Finder</button>
            </div>
          </div>
        )}

        {fda === false && fdaDismissed && (
          <button className="btn btn-ghost btn-sm mt-4" onClick={() => setFdaDismissed(false)}>Enable Full Disk Access…</button>
        )}

        {/* new scan */}
        <div className="mt-8 grid grid-cols-3 gap-2">
          <TargetCard icon={<Home className="w-4 h-4" />} title="Home folder" sub={shortPath(info?.home ?? '~', info?.home ?? '')} active={target === 'home'} onClick={() => setTarget('home')} />
          <TargetCard icon={<HardDrive className="w-4 h-4" />} title="Whole disk" sub="Everything on this Mac" active={target === 'disk'} onClick={() => setTarget('disk')} />
          <TargetCard icon={<FolderSearch className="w-4 h-4" />} title={folder ? folder.split('/').filter(Boolean).pop() ?? 'Folder' : 'A folder…'} sub={folder ? shortPath(folder, info?.home ?? '') : 'Pick any folder'} active={target === 'folder'} onClick={pickFolder} />
        </div>

        <div className="mt-4 flex items-center gap-6 text-[12.5px]">
          <div className="flex items-center gap-1 rounded-lg p-0.5" style={{ border: '1px solid var(--line)' }}>
            <ModeButton active={quick} onClick={() => setQuick(true)} title="Quick" sub="Measures everything, lists files over 256 KB" />
            <ModeButton active={!quick} onClick={() => setQuick(false)} title="Thorough" sub="Lists every single file" />
          </div>
          <AnimatePresence>
            {target === 'disk' && (
              <motion.button initial={{ opacity: 0, x: -6 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -6 }} className="flex items-center gap-2" style={{ color: admin ? 'var(--text)' : 'var(--muted)' }} onClick={() => setAdmin(!admin)}>
                <span className={`switch ${admin ? 'on' : ''}`} />
                <Shield className="w-3.5 h-3.5" /> Scan as administrator
              </motion.button>
            )}
          </AnimatePresence>
          <span className="flex-1" />
          <button className="btn btn-primary !px-4" onClick={begin}>
            Start {quick ? 'quick' : 'thorough'} scan
          </button>
        </div>
        {target === 'disk' && (
          <p className="text-[11.5px] mt-3 leading-relaxed" style={{ color: 'var(--dim)' }}>
            {admin
              ? 'macOS will ask for your password. Administrator scans can see other users, system caches, swap and hidden space that a normal scan reports as unreadable. Cleanup stays limited to your home folder.'
              : 'A normal whole-disk scan skips folders you cannot read. Turn on administrator mode to see them.'}{' '}
            External, network and cloud drives are never included.
          </p>
        )}

        {/* previous scans */}
        {active && (
          <div className="mt-10">
            <div className="label mb-2">Last scan</div>
            <div className="rounded-xl overflow-hidden" style={{ border: '1px solid var(--line)' }}>
              <ScanRow s={active} home={info?.home ?? ''} busy={busy === active.path} onOpen={() => openScan(active)} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

const TargetCard: React.FC<{ icon: React.ReactNode; title: string; sub: string; active: boolean; onClick: () => void }> = ({ icon, title, sub, active, onClick }) => (
  <button onClick={onClick} className="text-left rounded-xl p-3.5 transition-colors" style={{ border: `1px solid ${active ? 'var(--text)' : 'var(--line)'}`, background: active ? 'var(--bg-2)' : 'transparent' }}>
    <span className="flex items-center gap-2 text-[13px] font-medium">
      <span style={{ color: active ? 'var(--text)' : 'var(--muted)' }}>{icon}</span>
      {title}
    </span>
    <span className="block text-[11.5px] mt-1 truncate" style={{ color: 'var(--dim)' }}>{sub}</span>
  </button>
);

const ModeButton: React.FC<{ active: boolean; onClick: () => void; title: string; sub: string }> = ({ active, onClick, title, sub }) => (
  <button onClick={onClick} className="text-left px-3 py-1.5 rounded-md transition-colors" style={{ background: active ? 'var(--bg-3)' : 'transparent', color: active ? 'var(--text)' : 'var(--muted)' }} title={sub}>
    <span className="block text-[12.5px] font-medium">{title}</span>
    <span className="block text-[10.5px]" style={{ color: 'var(--dim)' }}>{sub}</span>
  </button>
);

const ScanRow: React.FC<{ s: ScanInfo; home: string; busy: boolean; onOpen: () => void }> = ({ s, home, busy, onOpen }) => {
  const label = s.root === '/' ? 'Whole disk' : s.root === home ? 'Home folder' : shortPath(s.root, home);
  return (
    <div className="group flex items-center gap-3 px-4 h-14 row-hover">
      <div className="flex-1 min-w-0">
        <div className="text-[13px] font-medium flex items-center gap-2">
          {label}
          <span className="chip">{s.mode === 'quick' ? 'quick' : 'thorough'}</span>
        </div>
        <div className="text-[11.5px] tnum mt-0.5" style={{ color: 'var(--dim)' }}>
          {s.finished ? relativeTime(s.finished) : 'incomplete'} · {s.entries ? `${formatNumber(s.entries)} entries` : '—'} · {formatBytes(s.size_bytes)} on disk
        </div>
      </div>
      <button className="btn btn-primary" onClick={onOpen} disabled={busy || !s.complete}>
        Continue <ChevronRight className="w-3.5 h-3.5" />
      </button>
    </div>
  );
};

const ProgressView: React.FC<{ onCancel: () => void }> = ({ onCancel }) => {
  const { scan, setScan, setStartScreen, info } = useStore();
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, []);
  const failed = scan?.done && !scan.ok;
  const elapsed = scan?.elapsed_ms ? scan.elapsed_ms / 1000 : tick;
  const rate = scan?.elapsed_ms && scan.entries ? Math.round(scan.entries / (scan.elapsed_ms / 1000)) : 0;
  const phase = scan?.phase ?? 'scan';
  const phaseLabel = phase === 'aggregate' ? 'Adding up folder sizes' : phase === 'write' ? 'Writing database' : phase === 'candidates' ? 'Finding cleanup candidates' : phase === 'unused' ? 'Checking for inactive files' : phase === 'done' ? 'Done' : 'Reading the filesystem';

  return (
    <div className="h-full flex items-center justify-center">
      <div className="w-[520px] px-8">
        <div className="flex items-center gap-3">
          {!failed && (
            <motion.span className="w-4 h-4 rounded-full flex-shrink-0" style={{ border: '1.5px solid var(--line-2)', borderTopColor: 'var(--text)' }} animate={{ rotate: 360 }} transition={{ repeat: Infinity, duration: 0.9, ease: 'linear' }} />
          )}
          <h2 className="text-[16px] font-semibold">{failed ? 'Scan stopped' : phaseLabel}</h2>
        </div>
        <div className="mt-6 grid grid-cols-3 gap-4">
          <Stat label="Entries" value={scan?.entries ? formatNumber(scan.entries) : '—'} />
          <Stat label="Measured" value={scan?.bytes ? formatBytes(scan.bytes) : '—'} />
          <Stat label="Elapsed" value={formatDuration(elapsed)} sub={rate ? `${formatNumber(rate)} / s` : undefined} />
        </div>
        <div className="mt-6 h-[3px] rounded-full overflow-hidden" style={{ background: 'var(--line)' }}>
          {!failed && <div className="h-full w-1/3 shimmer" />}
        </div>
        <div className="mt-3 mono text-[11px] truncate" style={{ color: failed ? 'var(--danger)' : 'var(--dim)' }}>
          {failed ? scan?.message : scan?.current ? shortPath(scan.current, info?.home ?? '') : scan?.message}
        </div>
        <div className="mt-8 flex items-center gap-2">
          {failed ? (
            <button className="btn" onClick={() => { setScan(null); setStartScreen(true); }}>Back</button>
          ) : (
            <button className="btn" onClick={onCancel}><X className="w-3.5 h-3.5" /> Cancel</button>
          )}
        </div>
      </div>
    </div>
  );
};

const Stat: React.FC<{ label: string; value: string; sub?: string }> = ({ label, value, sub }) => (
  <div>
    <div className="label">{label}</div>
    <div className="tnum text-[22px] font-medium mt-1 leading-none">{value}</div>
    {sub && <div className="tnum text-[11px] mt-1" style={{ color: 'var(--dim)' }}>{sub}</div>}
  </div>
);
