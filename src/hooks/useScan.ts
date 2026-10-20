import { useCallback, useEffect } from 'react';
import { useStore } from '../store';
import { api, errorText, onScanFallback, onScanProgress, onScanReclassified } from '../lib/api';

let listening = false;

export function useScan() {
  const { scan, setScan, toast, boot, reloadData } = useStore();

  useEffect(() => {
    if (listening) return;
    listening = true;
    let owner = true;
    let unlisten: (() => void) | null = null;
    let unlistenRules: (() => void) | null = null;
    let unlistenFallback: (() => void) | null = null;
    onScanFallback((reason) => useStore.getState().toast({ kind: 'info', title: 'Scanning without administrator rights', detail: `The administrator scan didn’t work: ${reason}` })).then((u) => (unlistenFallback = u));
    onScanProgress((p) => {
      // Only the walk sends the live preview; later phases keep showing the last one.
      const prev = useStore.getState().scan;
      if (!p.done && !p.top?.length && prev?.top?.length) p = { ...p, top: prev.top, junk: prev.junk };
      useStore.getState().setScan(p);
      if (p.done && p.ok) {
        setTimeout(() => useStore.getState().setScan(null), 1500);
      }
    }).then((u) => (unlisten = u));
    // The loaded scan was written by an older rule set and has just been updated in place; pull
    // the corrected categories in so views stop showing stale verdicts.
    onScanReclassified(() => {
      const st = useStore.getState();
      st.reloadData();
      st.refreshCurrent();
    }).then((u) => (unlistenRules = u));
    return () => {
      if (!owner) return;
      owner = false;
      unlisten?.();
      unlistenRules?.();
      unlistenFallback?.();
      listening = false;
    };
  }, []);

  const startScan = useCallback(
    async (root?: string, opts?: { quick?: boolean; admin?: boolean }) => {
      setScan({ phase: 'scan', message: opts?.admin ? 'Waiting for administrator approval…' : 'Starting scan…', entries: 0, dirs: 0, bytes: 0, current: '', elapsed_ms: 0, done: false, ok: true });
      try {
        const meta = await api.runScan(root, opts?.quick ?? true, opts?.admin ?? false);
        toast({ kind: 'success', title: 'Scan complete', detail: 'Disko is reviewing the results.' });
        await boot();
        await reloadData();
        // Hand the fresh scan to the agent for a supervised review.
        const st = useStore.getState();
        st.setStartScreen(false);
        if (!['explore', 'junk', 'suggestions', 'timeline'].includes(st.page)) st.setPage('explore');
        st.refreshScans();
        if (st.advanced) st.setAssistantOpen(true);
        window.dispatchEvent(
          new CustomEvent('disko:ask', {
            detail: `A fresh scan of ${meta.root} just finished (${meta.entries ?? 'unknown'} entries). Review it: read candidates.json and remnants.json from the reports folder and query the scan database for the largest folders, then explain in a few short paragraphs where the space went. Recommend a cleanup plan ranked by size and safety using the decision model: clearly disposable first, then leftovers of uninstalled apps, then items that need my confirmation with their recreate cost. Where a manager command is safer than moving a folder (Docker, Homebrew, pnpm, uv, simulators, model caches), give the command instead of a path. Put the exact paths in a disko-actions block. This is a review only: do not quarantine anything unless I ask.`,
          }),
        );
      } catch (e) {
        const msg = errorText(e);
        if (/cancel/i.test(msg)) setScan(null);
        else {
          toast({ kind: 'error', title: 'Scan failed', detail: msg });
          setScan({ phase: 'scan', message: msg, entries: 0, dirs: 0, bytes: 0, current: '', elapsed_ms: 0, done: true, ok: false });
        }
      }
    },
    [boot, reloadData, setScan, toast],
  );

  const pickAndScan = useCallback(async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const dir = await open({ directory: true, multiple: false, title: 'Choose a folder to scan' });
      if (typeof dir === 'string' && dir) await startScan(dir, { quick: true });
    } catch (e) {
      toast({ kind: 'error', title: 'Could not open folder picker', detail: errorText(e) });
    }
  }, [startScan, toast]);

  const cancelScan = useCallback(async () => {
    try {
      await api.cancelScan();
    } catch {}
  }, []);

  return { scan, scanning: !!scan && !scan.done, startScan, pickAndScan, cancelScan };
}
