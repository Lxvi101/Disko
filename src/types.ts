export type Category = 'rebuildable' | 'review' | 'keep' | 'protected';

export interface VolumeInfo {
  total: number;
  available: number;
  available_including_purgeable?: number;
  used: number;
}

export interface FileItem {
  id?: number | null;
  parent?: number | null;
  path: string;
  name: string;
  kind: string;
  total: number;
  allocated: number;
  logical: number;
  category: Category | string;
  reason: string;
  tag: string;
  action: ActionKind | string;
  cost: RecreateCost | string;
  tool?: string | null;
  mtime_ns?: number | null;
  error?: string | null;
  child_count?: number | null;
}

/** How a path should be removed. */
export type ActionKind = 'quarantine' | 'manager_command' | 'owner_app' | 'report_only';
export type RecreateCost = 'instant' | 'local-rebuild' | 'network-small' | 'network-large' | 'unknown' | 'irreplaceable';

export interface TreeNode {
  path: string;
  name: string;
  kind: string;
  total: number;
  category: string;
  reason: string;
  tag: string;
  action: ActionKind | string;
  children: TreeNode[];
  rest_total: number;
  rest_count: number;
}

export interface ScanMeta {
  root: string;
  started?: number | null;
  finished?: number | null;
  entries?: number | null;
  complete: boolean;
  db_path: string;
  coverage_issues: number;
}

export interface AppInfo {
  home: string;
  workspace: string;
  db_path: string;
  db_loaded: boolean;
  codex_path?: string | null;
  codex_version?: string | null;
  scan?: ScanMeta | null;
  volume: VolumeInfo;
}

export interface OverviewData {
  root: string;
  volume: VolumeInfo;
  items: FileItem[];
  db_loaded: boolean;
  entry_count: number;
  category_totals: Record<string, number>;
}

export interface CandidateItem {
  path: string;
  name: string;
  kind: string;
  total: number;
  category: string;
  reason: string;
  tag: string;
  action: ActionKind | string;
  cost: RecreateCost | string;
  tool?: string | null;
}

export interface RemnantMember {
  path: string;
  kind: 'cache' | 'http' | 'saved-state' | 'logs' | 'webkit' | 'state' | 'container' | 'group' | 'prefs' | 'scripts' | 'cookies' | string;
  total: number;
  newest_mtime?: number | null;
  preselect: boolean;
  persistent: boolean;
}

export interface RemnantGroup {
  id: string;
  name: string;
  score: number;
  confidence: 'likely' | 'confirm' | string;
  total: number;
  newest_mtime?: number | null;
  members: RemnantMember[];
  evidence: string[];
}

export interface OrphanAgent {
  path: string;
  label: string;
  program: string;
  reason: string;
}

export interface RemnantsReport {
  schema: number;
  rules: number;
  generated_at: number;
  db: string;
  installed_apps: number;
  groups: RemnantGroup[];
  agents: OrphanAgent[];
}

export interface UnusedItem {
  path: string;
  name: string;
  logical_bytes: number;
  allocated_bytes: number;
  days_inactive: number;
  category: string;
  reason: string;
  last_used?: string | null;
  last_accessed?: string | null;
  modified?: string | null;
  evidence?: string | null;
  kind: 'file' | 'app' | string;
}

export interface CodexModel {
  slug: string;
  display_name: string;
  description: string;
  efforts: string[];
  default_effort: string;
  priority: number;
}

export interface CodexModels {
  default_model: string;
  default_effort: string;
  models: CodexModel[];
}

export interface ContextItem {
  path: string;
  total: number;
  category: string;
  kind: string;
  tag?: string;
  action?: string;
}

export interface HistoryTurn {
  role: 'user' | 'assistant';
  text: string;
}

export interface ChatContext {
  page: string;
  current_path: string;
  items: ContextItem[];
  staged: ContextItem[];
  history: HistoryTurn[];
}

export interface CodexResult {
  request_id: string;
  success: boolean;
  text: string;
  error?: string | null;
  usage?: Record<string, number> | null;
}

export interface CodexEventPayload {
  request_id: string;
  event: any;
}

export interface Suggestion {
  path: string;
  bytes?: number;
  reason: string;
  confidence: 'high' | 'medium' | 'low' | string;
  /** `quarantine` (default) can be collected; `command` shows the manager command instead. */
  action?: 'quarantine' | 'command' | string;
  command?: string;
}

export interface ActivityItem {
  id: string;
  type: 'command' | 'reasoning' | 'note' | 'error';
  title: string;
  detail?: string;
  status?: 'running' | 'done' | 'failed';
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  text: string;
  createdAt: number;
  model?: string;
  streaming?: boolean;
  activity?: ActivityItem[];
  suggestions?: Suggestion[];
  error?: string | null;
  usage?: Record<string, number> | null;
}

export interface StagedItem {
  path: string;
  name: string;
  kind: string;
  total: number;
  category: string;
  reason: string;
  tag?: string;
  action?: ActionKind | string;
  tool?: string | null;
  source: 'explore' | 'suggestion' | 'inactive' | 'assistant' | 'search' | 'remnant';
}

export interface QuarantineInput {
  path: string;
  size: number;
}

export interface QuarantineResult {
  journal_path: string;
  moved_items: string[];
  skipped: string[];
  total_bytes: number;
}

export interface QuarantineEntry {
  original: string;
  quarantined: string;
  size: number;
  present: boolean;
}

export interface QuarantineJournal {
  id: string;
  path: string;
  created_at: number;
  items: QuarantineEntry[];
  total_bytes: number;
  purged: boolean;
}

export interface ScanProgress {
  phase: string;
  message: string;
  entries: number;
  dirs: number;
  bytes: number;
  current: string;
  elapsed_ms: number;
  done: boolean;
  ok: boolean;
}

export interface ScanInfo {
  path: string;
  file: string;
  root: string;
  started?: number | null;
  finished?: number | null;
  entries?: number | null;
  rows?: number | null;
  mode: string;
  complete: boolean;
  size_bytes: number;
  active: boolean;
}

export type Page = 'apps' | 'explore' | 'junk' | 'suggestions' | 'timeline' | 'quarantine';
export type VizMode = 'sunburst' | 'treemap' | 'list';

export interface XcodeItem {
  path: string; name: string; group: string; total: number;
  modified: number | null; last_booted: string | null; subtitle: string;
  badges: string[]; blocked: string | null; partial: boolean;
}
export interface XcodeInventory { items: XcodeItem[]; warnings: string[] }
export interface DeleteResult {
  deleted_items: string[]; failures: { path: string; error: string }[]; total_bytes: number;
}
export type CleanupMode = 'quarantine' | 'delete';

export interface AppStorageInventory { total: number; potential: number; partial: boolean; items: XcodeItem[]; warnings: string[] }

/** The outermost folder of a generated or disposable tree: a node_modules, a venv, an app's cache. */
export interface JunkItem {
  path: string; name: string; kind: string; total: number; mtime: number | null;
  category: string; reason: string; tag: string; action: ActionKind | string; cost: RecreateCost | string; tool?: string | null;
}
export interface DupFile { path: string; name: string; mtime: number | null }
export interface DupGroup { size: number; allocated: number; files: DupFile[] }

export type TimelineLane = 'app' | 'project' | 'file' | 'download' | 'developer' | 'backup';
/** Anything with an age: an app by last use, a project by last git activity, a file by last open. */
export interface OldItem {
  path: string; name: string; lane: TimelineLane; label: string; total: number;
  /** Newest evidence of use in seconds; null when macOS keeps no record. */
  date: number | null;
  date_kind: 'opened' | 'last seen' | 'worked on' | 'modified' | 'added' | 'booted' | 'installed' | 'backed up' | 'created' | 'last changed' | string;
  detail: string; category: string; action: string; tool?: string | null;
  /** Generated folders inside a project, as [path, bytes]. */
  junk: [string, number][];
}
