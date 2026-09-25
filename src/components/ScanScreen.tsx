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

  const targetName = target === 'home' ? 'home folder' : target === 'disk' ? 'whole disk' : folder ? folder.split('/').filter(Boolean).pop() ?? 'folder' : 'folder';

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-[560px] mx-auto px-8 pt-14 pb-16">
        {active && (
          <section className="mb-10">
            <h1 className="text-[22px] font-semibold tracking-tight">Welcome back</h1>
            <p className="text-[13px] mt-1" style={{ color: 'var(--muted)' }}>Pick up where you left off, or start a fresh scan below.</p>
            <LastScan s={active} home={info?.home ?? ''} busy={busy === active.path} onOpen={() => openScan(active)} />
          </section>
        )}

        <h2 className={active ? 'text-[15px] font-semibold' : 'text-[22px] font-semibold tracking-tight'}>{active ? 'New scan' : 'What should Disko scan?'}</h2>
        <p className="text-[13px] mt-1" style={{ color: 'var(--muted)' }}>Disko only reads sizes and dates. Nothing is moved or deleted.</p>

        {fda === false && !fdaDismissed && (
          <div className="mt-5 rounded-xl p-4 flex items-start gap-3" style={{ border: '1px solid var(--line-2)', background: 'var(--bg-2)' }}>
            <Lock className="w-4 h-4 mt-0.5 flex-shrink-0" style={{ color: 'var(--muted)' }} />
            <div className="flex-1 min-w-0 text-[12.5px]">
              <div className="font-medium">Allow Full Disk Access for complete results</div>
              <p className="mt-1 leading-relaxed" style={{ color: 'var(--muted)' }}>
                Without it, some protected folders are skipped. Turn on Disko in System Settings → Privacy &amp; Security → Full Disk Access.
              </p>
              <div className="mt-3 flex flex-wrap items-center gap-2">
                <button className="btn btn-primary btn-sm" disabled={fdaBusy} onClick={requestAccess}>{fdaBusy ? 'Checking…' : fdaRequested ? 'Open Settings again' : 'Open Settings'}</button>
                <button className="btn btn-ghost btn-sm" disabled={fdaBusy} onClick={checkAccess}>I’ve turned it on</button>
                <button className="btn btn-ghost btn-sm" onClick={() => api.revealRunningApp().catch(e => setFdaMessage(errorText(e)))}>Show Disko in Finder</button>
                <span className="flex-1" />
                <button className="btn btn-ghost btn-sm" disabled={fdaBusy} onClick={() => setFdaDismissed(true)}>Not now</button>
              </div>
              {fdaMessage && <p role="status" className="mt-3 leading-relaxed" style={{ color: 'var(--muted)' }}>{fdaMessage}</p>}
            </div>
          </div>
        )}

        <div role="radiogroup" aria-label="What to scan" className="mt-5 rounded-xl overflow-hidden" style={{ border: '1px solid var(--line)', background: 'var(--bg-2)' }}>
          <TargetRow icon={<Home className="w-[18px] h-[18px]" />} title="Home folder" sub="Your documents, apps’ data and caches. Best place to start." active={target === 'home'} onClick={() => setTarget('home')} />
          <TargetRow icon={<HardDrive className="w-[18px] h-[18px]" />} title="Whole disk" sub="Everything on this Mac, including system files." active={target === 'disk'} onClick={() => setTarget('disk')} />
          <TargetRow icon={<FolderSearch className="w-[18px] h-[18px]" />} title={folder ? folder.split('/').filter(Boolean).pop() ?? 'Folder' : 'A specific folder…'} sub={folder ? shortPath(folder, info?.home ?? '') : 'Choose any folder on this Mac.'} active={target === 'folder'} onClick={pickFolder} />
        </div>

        <div className="mt-4 space-y-2.5">
          <Check on={!quick} onChange={(v) => setQuick(!v)} label="List every file" hint="Slower. Otherwise files under 256 KB are counted but not listed." />
          <AnimatePresence initial={false}>
            {target === 'disk' && (
              <motion.div initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }} className="overflow-hidden">
                <Check on={admin} onChange={setAdmin} label={<span className="inline-flex items-center gap-1.5"><Shield className="w-3.5 h-3.5" /> Scan as administrator</span>} hint="Also measures other users, system caches and swap. macOS will ask for your password. Cleanup stays limited to your home folder." />
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        <button className={`btn ${active ? '' : 'btn-primary'} w-full mt-6 !py-2.5 !text-[13px]`} onClick={begin}>
          {target === 'folder' && !folder ? 'Choose a folder…' : `Scan ${targetName}`}
        </button>
        {target === 'disk' && (
          <p className="text-[11.5px] mt-3 text-center" style={{ color: 'var(--dim)' }}>External, network and cloud drives are not included.</p>
        )}
        {fda === false && fdaDismissed && (
          <button className="btn btn-ghost btn-sm mt-3 mx-auto flex" onClick={() => setFdaDismissed(false)}><Lock className="w-3 h-3" /> Enable Full Disk Access…</button>
        )}
      </div>
    </div>
  );
};

const TargetRow: React.FC<{ icon: React.ReactNode; title: string; sub: string; active: boolean; onClick: () => void }> = ({ icon, title, sub, active, onClick }) => (
  <button role="radio" aria-checked={active} onClick={onClick} className="scan-target w-full flex items-center gap-3.5 px-4 py-3.5 text-left transition-colors">
    <span className="w-9 h-9 rounded-[10px] flex items-center justify-center flex-shrink-0" style={{ background: active ? 'var(--text)' : 'var(--bg-3)', color: active ? 'var(--bg)' : 'var(--muted)' }}>{icon}</span>
    <span className="flex-1 min-w-0">
      <span className="block text-[13.5px] font-medium">{title}</span>
      <span className="block text-[12px] mt-0.5 truncate" style={{ color: 'var(--muted)' }}>{sub}</span>
    </span>
    <span className="w-[18px] h-[18px] rounded-full flex-shrink-0 flex items-center justify-center" style={{ border: `1.5px solid ${active ? 'var(--text)' : 'var(--line-2)'}` }}>
      {active && <span className="w-2 h-2 rounded-full" style={{ background: 'var(--text)' }} />}
    </span>
  </button>
);

const Check: React.FC<{ on: boolean; onChange: (v: boolean) => void; label: React.ReactNode; hint: string }> = ({ on, onChange, label, hint }) => (
  <label className="flex items-start gap-2.5 cursor-default px-1">
    <input type="checkbox" className="mt-[3px]" checked={on} onChange={(e) => onChange(e.target.checked)} />
    <span className="text-[12.5px]">
      <span className="font-medium">{label}</span>
      <span className="block text-[11.5px] mt-0.5 leading-relaxed" style={{ color: 'var(--dim)' }}>{hint}</span>
    </span>
  </label>
);

const LastScan: React.FC<{ s: ScanInfo; home: string; busy: boolean; onOpen: () => void }> = ({ s, home, busy, onOpen }) => {
  const isDisk = s.root === '/';
  const label = isDisk ? 'Whole disk' : s.root === home ? 'Home folder' : shortPath(s.root, home);
  return (
    <button className="scan-last group mt-5 w-full flex items-center gap-3.5 rounded-xl px-4 py-3.5 text-left transition-colors" onClick={onOpen} disabled={busy || !s.complete}>
      <span className="w-9 h-9 rounded-[10px] flex items-center justify-center flex-shrink-0" style={{ background: 'var(--bg-3)', color: 'var(--text)' }}>
        {isDisk ? <HardDrive className="w-[18px] h-[18px]" /> : s.root === home ? <Home className="w-[18px] h-[18px]" /> : <FolderSearch className="w-[18px] h-[18px]" />}
      </span>
      <span className="flex-1 min-w-0">
        <span className="block text-[13.5px] font-medium truncate">{label}</span>
        <span className="block text-[12px] tnum mt-0.5" style={{ color: 'var(--muted)' }}>
          {s.complete ? `Scanned ${relativeTime(s.finished)}` : 'Incomplete'}{s.entries ? ` · ${formatNumber(s.entries)} items` : ''}{s.mode === 'quick' ? '' : ' · every file'}
        </span>
      </span>
      <span className="btn btn-primary pointer-events-none">{busy ? 'Opening…' : 'Open'} <ChevronRight className="w-3.5 h-3.5" /></span>
    </button>
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
