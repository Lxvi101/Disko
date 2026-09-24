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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
