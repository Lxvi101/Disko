import React, { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Download, X } from 'lucide-react';
import type { Update } from '@tauri-apps/plugin-updater';
import { isTauri } from '../../lib/api';

// Checks GitHub Releases once on launch. Nothing downloads until the user says so.
export const UpdatePrompt: React.FC = () => {
  const [update, setUpdate] = useState<Update | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (import.meta.env.DEV || !isTauri) return;
    import('@tauri-apps/plugin-updater').then(({ check }) => check()).then(setUpdate).catch(() => {});
  }, []);

  const install = async () => {
    if (!update) return;
    setError(null);
    setProgress(0);
    let total = 0;
    let done = 0;
    try {
      await update.downloadAndInstall((e) => {
        if (e.event === 'Started') total = e.data.contentLength ?? 0;
        else if (e.event === 'Progress') {
          done += e.data.chunkLength;
          if (total) setProgress(done / total);
        }
      });
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      setProgress(null);
      setError(String(e));
    }
  };

  return (
    <AnimatePresence>
      {update && (
        <motion.div
          initial={{ opacity: 0, y: -16 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -16 }}
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          className="fixed top-[76px] right-5 z-[100] w-[330px] glass-strong rounded-2xl p-4 flex items-start gap-3"
        >
          <Download size={20} className="mt-0.5 shrink-0" style={{ color: 'var(--accent)' }} />
          <div className="flex-1 min-w-0">
            <div className="text-[15px] font-semibold">Disko {update.version} is here</div>
            <div className="text-[13px] mt-0.5 leading-snug break-words" style={{ color: error ? 'var(--danger)' : 'var(--muted)' }}>
              {error ?? (progress !== null ? `Downloading… ${Math.round(progress * 100)}%` : `You have ${update.currentVersion}.`)}
            </div>
            {progress === null && (
              <button className="btn btn-primary mt-3 !h-9" onClick={install}>{error ? 'Try again' : 'Install and restart'}</button>
            )}
          </div>
          {progress === null && (
            <button className="btn btn-ghost btn-icon !w-8 !h-8 -mr-1 -mt-1" aria-label="Later" onClick={() => setUpdate(null)}>
              <X size={16} />
            </button>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
};
