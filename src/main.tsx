import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { isTauri } from './lib/api';
import './index.css';

if (isTauri) {
  // Mirror page errors into the app log so crashes are debuggable without devtools.
  const log = (level: string, message: string) => {
    import('@tauri-apps/api/core').then(({ invoke }) => invoke('webview_log', { level, message })).catch(() => {});
  };
  window.addEventListener('error', (e) => log('error', `${e.message} @ ${e.filename}:${e.lineno}`));
  window.addEventListener('unhandledrejection', (e) => log('rejection', String(e.reason?.stack ?? e.reason)));
}

// WKWebView ignores `-webkit-app-region`, so `.drag-region` areas move the window by hand.
// Anything interactive, or inside `.no-drag`, keeps its own clicks; double-click zooms like a title bar.
const NOT_DRAGGABLE = '.no-drag, button, a, input, textarea, select, [role=button], [role=checkbox], [contenteditable]';
window.addEventListener('mousedown', (e) => {
  const el = e.target as Element | null;
  if (e.button !== 0 || !el?.closest?.('.drag-region') || el.closest(NOT_DRAGGABLE)) return;
  e.preventDefault();
  if (!isTauri) {
    (window as unknown as { __lastDrag?: number }).__lastDrag = Date.now();
    return;
  }
  import('@tauri-apps/api/window')
    .then(({ getCurrentWindow }) => {
      const w = getCurrentWindow();
      return e.detail === 2 ? w.toggleMaximize() : w.startDragging();
    })
    .catch(() => {});
});

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
