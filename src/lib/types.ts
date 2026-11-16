// Mirrors src-tauri/crates/core/src/types.rs (snake_case on purpose) and docs/contract.md.

export type Category =
  | 'build' | 'cache' | 'packages' | 'developer' | 'leftovers' | 'old-versions' | 'installers'
  | 'logs' | 'agent-data' | 'containers' | 'extras' | 'forgotten' | 'trash';

export type Safety = 'safe' | 'review';
export type Source = 'builtin' | 'user' | 'agent' | 'community';

export type Action =
  | { type: 'delete' }
  | { type: 'trash' }
  | { type: 'command'; argv: string[]; label: string };

export interface Finding {
  id: string;
  rule: string;
  category: Category;
  title: string;
  detail?: string;
  icon: string;
  paths: string[];
  bytes: number;
  last_used?: number;
  safety: Safety;
  action: Action;
  regrows: boolean;
  why: string;
  busy?: string;
  admin?: boolean;
  source?: Source;
}

export interface TreeNode {
  name: string;
  path: string;
  /** "dir" | "file" | "other" */
  kind: string;
  bytes: number;
  files: number;
  newest?: number;
  clutter?: Category;
  error?: string;
  children?: TreeNode[];
}

export interface HiddenSpace {
  /** "snapshots" | "system" | "vm" | "unreadable" | "purgeable" | "other" */
  kind: string;
  label: string;
  bytes: number;
  why: string;
}

export interface VolumeInfo {
  name: string;
  total: number;
  used: number;
  free: number;
  purgeable: number;
  scanned: number;
  hidden: HiddenSpace[];
}

export interface ScanProgress {
  /** "probes" | "walk" | "done" */
  phase: string;
  files: number;
  dirs: number;
  bytes: number;
  elapsed_ms: number;
  current: string;
  fraction: number;
}

export interface Trashed { from: string; to: string; bytes: number }
export interface CleanFailure { path: string; error: string }

export interface CleanResult {
  freed: number;
  removed: string[];
  trashed: Trashed[];
  failed: CleanFailure[];
  skipped: CleanFailure[];
}

export interface BootInfo {
  version: string;
  home: string;
  full_disk_access: boolean;
  volume: VolumeInfo;
  snapshot_age_s: number | null;
}

export interface AppInfo {
  name: string;
  bundle_id: string;
  path: string;
  version?: string;
  icon: string;
  app_bytes: number;
  data_bytes: number;
  data_paths: string[];
  last_used?: number;
  running: boolean;
  apple: boolean;
  /** Disko has a storage page for it (`app_storage`): Xcode, Adobe, browsers, Electron apps. */
  storage?: boolean;
}

/** One app's storage page: everything it keeps that you can sort out item by item. */
export interface AppStorage {
  app: { name: string; path: string; icon: string; version?: string; profile: 'xcode' | 'adobe' | 'browser' | 'electron' | string };
  bytes: number;
  groups: StorageGroup[];
}

export interface StorageGroup {
  /** "simulators" | "runtimes" | "device-support" | "derived-data" | "archives" | "caches" | "xcodes" | "media-cache" | "old-versions" | "profiles" | "cli-versions" */
  id: string;
  title: string;
  /** Glyph key. */
  icon: string;
  bytes: number;
  /** Fine print for the whole group, only where it matters. */
  note?: string;
  /** Findings: removed through `clean` by id like any other. */
  items: Finding[];
}

export interface AgentSession {
  id: string;
  title?: string;
  bytes: number;
  started?: number;
  last_used?: number;
  paths: string[];
}

export interface AgentProject {
  name: string;
  path?: string;
  bytes: number;
  last_used?: number;
  sessions: AgentSession[];
}

export interface AgentTool {
  tool: string;
  name: string;
  icon: string;
  bytes: number;
  projects: AgentProject[];
}

export interface KeepEntry { id: string; pattern: string; note?: string; source: 'user' | 'agent'; added: number }

export interface LearnedRule {
  id: string;
  pattern: string;
  category: Category;
  title: string;
  why: string;
  regrows: boolean;
  safety: Safety;
  source: 'user' | 'agent' | 'community';
  added: number;
  shared: boolean;
}

export interface Routine {
  id: string;
  title: string;
  select: { categories?: Category[]; rules?: string[] };
  every_days: number;
  min_age_days: number;
  enabled: boolean;
  last_run?: number;
  last_freed?: number;
}

export interface Memory {
  keep: KeepEntry[];
  rules: LearnedRule[];
  routines: Routine[];
  community: { enabled: boolean; rules: number; updated?: number };
}

export type MemoryOp =
  | { type: 'keep'; pattern: string; note?: string }
  | { type: 'unkeep'; id: string }
  | { type: 'add-rule'; rule: LearnedRule }
  | { type: 'remove-rule'; id: string }
  | { type: 'set-routine'; routine: Routine }
  | { type: 'remove-routine'; id: string }
  | { type: 'community'; enabled: boolean };

/** Added by the frontend (not yet in docs/contract.md): MCP clients Disko can register itself with. */
export interface McpClient {
  id: string;
  name: string;
  /** The client is installed on this Mac. */
  present: boolean;
  /** Disko's MCP server is registered in it. */
  connected: boolean;
}

export interface Events {
  'scan:progress': ScanProgress;
  'scan:tree': TreeNode;
  'clutter:upsert': Finding[];
  'clutter:remove': string[];
  'tree:changed': { paths: string[] };
  volume: VolumeInfo;
  memory: Memory;
}
