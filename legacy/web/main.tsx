import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./index.css";
import { invoke } from "@tauri-apps/api/core";

// Mirror page errors into a log file next to the workspace so crashes are debuggable without devtools.
const log = (level: string, message: string) => { try { invoke("webview_log", { level, message }); } catch {} };
window.addEventListener("error", (e) => log("error", `${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => log("rejection", String((e as PromiseRejectionEvent).reason?.stack ?? (e as PromiseRejectionEvent).reason)));
const origError = console.error.bind(console);
console.error = (...a: unknown[]) => { log("console", a.map((x) => (x instanceof Error ? x.stack ?? x.message : typeof x === "string" ? x : JSON.stringify(x))).join(" ")); origError(...a); };
const origWarn = console.warn.bind(console);
console.warn = (...a: unknown[]) => { log("warn", a.map((x) => (typeof x === "string" ? x : JSON.stringify(x))).join(" ")); origWarn(...a); };

// WKWebView ignores `-webkit-app-region`, so `.drag-region` bars move the window by hand.
// Anything interactive, or inside `.no-drag`, keeps its own clicks; double-click zooms like a title bar.
const NOT_DRAGGABLE = ".no-drag, button, a, input, textarea, select, [role=button], [contenteditable]";
window.addEventListener("mousedown", (e) => {
  const el = e.target as Element | null;
  if (e.button !== 0 || !el?.closest?.(".drag-region") || el.closest(NOT_DRAGGABLE)) return;
  e.preventDefault();
  import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
    const w = getCurrentWindow();
    return e.detail === 2 ? w.toggleMaximize() : w.startDragging();
  }).catch((err) => log("error", `window drag: ${err}`));
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
