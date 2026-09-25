import React, { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Download, X } from 'lucide-react';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

// Checks GitHub Releases once on launch. Nothing downloads until the user says so.
export const UpdatePrompt: React.FC = () => {
  const [update, setUpdate] = useState<Update | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    check().then(setUpdate).catch(() => {});
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
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 20 }}
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          className="fixed bottom-4 right-4 z-[100] w-[320px] glass-strong rounded-2xl p-3 flex items-start gap-3"
        >
          <Download className="w-4 h-4 mt-0.5 flex-shrink-0" style={{ color: '#8b7cf6' }} />
          <div className="flex-1 min-w-0">
            <div className="text-[12.5px] font-semibold">Disko {update.version} is available</div>
            <div className="text-[11.5px] mt-0.5 leading-snug break-words" style={{ color: error ? '#fb7185' : 'var(--muted)' }}>
              {error ?? (progress !== null ? `Downloading… ${Math.round(progress * 100)}%` : `You have ${update.currentVersion}.`)}
            </div>
            {progress === null && (
              <button className="btn mt-2 !py-1 !px-2.5 text-[11.5px]" onClick={install}>
                {error ? 'Try again' : 'Install and restart'}
              </button>
            )}
          </div>
          {progress === null && (
            <button className="btn btn-ghost !p-0.5 -mr-1 -mt-0.5" onClick={() => setUpdate(null)}>
              <X className="w-3.5 h-3.5" />
            </button>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
};
