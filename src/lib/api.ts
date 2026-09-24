import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppInfo,
  AppStorageInventory,
  XcodeInventory,
  DeleteResult,
  CandidateItem,
  ChatContext,
  CodexEventPayload,
  CodexModels,
  CodexResult,
  FileItem,
  OverviewData,
  QuarantineInput,
  QuarantineJournal,
  QuarantineResult,
  ScanInfo,
  ScanMeta,
  ScanProgress,
  TreeNode,
  UnusedItem,
  VolumeInfo,
  RemnantsReport,
} from '../types';

export const isTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const api = {
  appInfo: () => invoke<AppInfo>('get_app_info'),
  overview: () => invoke<OverviewData>('load_disk_overview'),
  children: (path: string, limit = 250) => invoke<FileItem[]>('query_folder_children', { path, limit }),
  subtree: (path: string, depth = 3) => invoke<TreeNode>('get_subtree', { path, depth }),
  pathInfo: (path: string) => invoke<FileItem | null>('get_path_info', { path }),
  search: (query: string, limit = 40) => invoke<FileItem[]>('search_entries', { query, limit }),
  largestFiles: (limit = 40) => invoke<FileItem[]>('get_largest_files', { limit }),
  candidates: () => invoke<CandidateItem[]>('get_candidates'),
  unusedFiles: () => invoke<UnusedItem[]>('get_unused_files'),
  unusedApps: () => invoke<UnusedItem[]>('get_unused_apps'),
  remnants: (force = false) => invoke<RemnantsReport>('get_remnants', { force }),
  models: () => invoke<CodexModels>('list_codex_models'),
  chat: (args: { requestId: string; prompt: string; model: string; effort: string; context: ChatContext }) =>
    invoke<CodexResult>('codex_chat', args),
  cancelChat: (requestId: string) => invoke<boolean>('codex_cancel', { requestId }),
  runScan: (root?: string, quick?: boolean, admin?: boolean) => invoke<ScanMeta>('run_scan', { root, quick, admin }),
  listScans: () => invoke<ScanInfo[]>('list_scans'),
  setActiveScan: (path: string) => invoke<void>('set_active_scan', { path }),
  deleteScan: (path: string) => invoke<void>('delete_scan', { path }),
  cancelScan: () => invoke<boolean>('cancel_scan'),
  reveal: (path: string) => invoke<void>('reveal_in_finder', { path }),
  open: (path: string) => invoke<void>('open_path', { path }),
  preview: (path: string) => invoke<void>('preview_path', { path }),
  openTerminal: (path: string) => invoke<void>('open_in_terminal', { path }),
  quarantine: (items: QuarantineInput[]) => invoke<QuarantineResult>('safe_quarantine_items', { items }),
  appStorage: (appId: string) => invoke<AppStorageInventory>('get_app_cleanup_storage', { appId }),
  xcodeStorage: () => invoke<XcodeInventory>('get_xcode_storage'),
  deleteItems: (items: QuarantineInput[]) => invoke<DeleteResult>('delete_items', { items }),
  journals: () => invoke<QuarantineJournal[]>('list_quarantine_journals'),
  restore: (journalPath: string) => invoke<number>('restore_journal', { journalPath }),
  purge: (journalPath: string) => invoke<number>('purge_journal', { journalPath }),
  deleteJournal: (journalPath: string) => invoke<void>('delete_journal', { journalPath }),
  volume: () => invoke<VolumeInfo>('get_volume'),
  coverageIssues: () => invoke<number>('get_coverage_issue_count'),
  fullDiskAccess: () => invoke<boolean>('check_full_disk_access'),
  requestFullDiskAccess: () => invoke<boolean>('request_full_disk_access'),
  revealRunningApp: () => invoke<void>('reveal_running_app'),
  launchOptions: () => invoke<{ start_page?: string | null; start_path?: string | null; advanced?: boolean | null; autoscan?: string | null }>('get_launch_options'),
};

export const onCodexEvent = (cb: (p: CodexEventPayload) => void): Promise<UnlistenFn> =>
  listen<CodexEventPayload>('codex-event', (e) => cb(e.payload));

export const onScanProgress = (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
  listen<ScanProgress>('scan-progress', (e) => cb(e.payload));

/// Fires once when a scan written by an older rule set has been brought up to date in place.
export const onScanReclassified = (cb: (rows: number) => void): Promise<UnlistenFn> =>
  listen<number>('scan-reclassified', (e) => cb(e.payload));

export function errorText(err: unknown): string {
  if (typeof err === 'string') return err;
  if (err && typeof err === 'object' && 'message' in err) return String((err as any).message);
  return String(err);
}
