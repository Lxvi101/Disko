// A fake, realistic Mac for running the UI in a plain browser (`bun run dev`). It implements every
// command and event in docs/contract.md: an ~8 s streaming scan that grows the map, findings that
// arrive over time, removals that shrink the tree at once, Trash + Undo, apps, agent sessions,
// memory and MCP clients. Nothing here touches the real disk.
import type {
  AgentProject, AgentSession, AgentTool, AppInfo, BootInfo, Category, CleanResult, Events, Finding,
  HiddenSpace, McpClient, Memory, MemoryOp, ScanProgress, Trashed, TreeNode, VolumeInfo,
} from './types';

const HOME = '/Users/alex';
const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
const GB = 1024 ** 3;
const MB = 1024 ** 2;
const SCAN_MS = 8000;
const PROBE_MS = 900;

// ---------- deterministic randomness ----------
let seed = 7;
const rnd = () => {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const pick = <T,>(a: T[]) => a[Math.floor(rnd() * a.length)];

// ---------- the fake disk ----------
interface MNode {
  name: string;
  path: string;
  kind: 'dir' | 'file';
  own: number;
  children: MNode[];
  parent: MNode | null;
  mtime: number;
  clutter?: Category;
  error?: string;
  /** When the walk reaches it (ms after scan start). */
  t: number;
}

type Spec = { n: string; b?: number; c?: Category; age?: number; kids?: Spec[]; err?: string; filler?: [number, number] };
const F = (n: string, b: number, age = 30, c?: Category): Spec => ({ n, b, age, c });
const D = (n: string, kids: Spec[], c?: Category, age?: number): Spec => ({ n, kids, c, age });
/** A folder of `count` generated files totalling about `bytes`. */
const bulk = (n: string, bytes: number, count = 24, c?: Category, age = 40, ext = 'dat'): Spec => ({
  n, c, age, kids: Array.from({ length: count }, (_, i) => F(`${n.replace(/\W/g, '').slice(0, 8).toLowerCase() || 'item'}-${(i + 1).toString(16).padStart(3, '0')}.${ext}`, (bytes / count) * (0.3 + rnd() * 1.4), age * (0.5 + rnd()))),
});

const NPM = ['react', 'react-dom', 'typescript', 'esbuild', '@next', 'next', '@swc', 'webpack', 'lucide-react', '@babel', 'framer-motion', 'vite', 'rollup', '@types', 'eslint', 'prettier', 'tailwindcss', 'zod', 'lodash', 'date-fns', 'sharp', '@img', 'playwright-core', 'convex'];
const nodeModules = (gb: number, age: number): Spec => D('node_modules', NPM.slice(0, 10 + Math.floor(rnd() * 14)).map((p, i) => bulk(p, (gb * GB) * (i < 3 ? 0.18 : 0.04) * (0.5 + rnd()), 6, undefined, age, 'js')), 'build', age);
const src = (mb: number, age: number) => D('src', [bulk('components', mb * MB * 0.5, 18, undefined, age, 'tsx'), bulk('lib', mb * MB * 0.3, 10, undefined, age, 'ts'), F('main.tsx', 8000, age)], undefined, age);
const nodeProject = (n: string, nm: number, age: number, extra: Spec[] = []) => D(n, [nodeModules(nm, age), src(4 + rnd() * 20, age), F('package.json', 3000, age), F('bun.lock', 300000, age), D('.git', [bulk('objects', (40 + rnd() * 200) * MB, 12, undefined, age, 'pack')]), ...extra], undefined, age);
const rustTarget = (gb: number, age: number) => D('target', [D('debug', [bulk('deps', gb * GB * 0.55, 20, undefined, age, 'rlib'), bulk('incremental', gb * GB * 0.25, 10, undefined, age), bulk('build', gb * GB * 0.08, 8, undefined, age)]), D('release', [bulk('deps', gb * GB * 0.1, 10, undefined, age, 'rlib')])], 'build', age);
const rustProject = (n: string, gb: number, age: number, extra: Spec[] = []) => D(n, [rustTarget(gb, age), D('src', [bulk('bin', 2 * MB, 6, undefined, age, 'rs')], undefined, age), F('Cargo.toml', 2000, age), F('Cargo.lock', 90000, age), D('.git', [bulk('objects', (60 + rnd() * 300) * MB, 12, undefined, age, 'pack')]), ...extra], undefined, age);

const app = (n: string, gb: number, age = 20): Spec => D(`${n}.app`, [D('Contents', [bulk('Frameworks', gb * GB * 0.6, 8, undefined, age, 'framework'), bulk('Resources', gb * GB * 0.35, 12, undefined, age, 'car'), D('MacOS', [F(n.split(' ')[0], gb * GB * 0.05, age)])])], undefined, age);

const claudeProjects = ['-Users-alex-Documents-Development-Disko', '-Users-alex-Development-atlas', '-Users-alex-Documents-Development-MealClub', '-Users-alex-Development-openrive', '-Users-alex-Documents-Development-horizon', '-Users-alex-Development-rive-lab', '-Users-alex', '-Users-alex-Documents-Development-vtf3'];
const sessionFiles = (count: number, mb: number, maxAge: number) => Array.from({ length: count }, (_, i) => F(`${uuid()}.jsonl`, mb * MB * (0.1 + rnd() * 2) * (i === 0 ? 4 : 1), Math.floor(rnd() * maxAge)));
function uuid() {
  const h = () => Math.floor(rnd() * 0xffff).toString(16).padStart(4, '0');
  return `${h()}${h()}-${h()}-${h()}-${h()}-${h()}${h()}${h()}`;
}

function spec(): Spec {
  return D('', [
    D('Applications', [
      app('Xcode', 14.2, 3), app('Final Cut Pro', 6.1, 40), app('Adobe Premiere Pro 2026', 4.3, 90), app('Adobe After Effects 2026', 3.9, 120),
      app('Adobe Photoshop 2026', 3.2, 12), app('Adobe Illustrator 2026', 2.6, 200), app('Microsoft Word', 2.3, 60), app('Microsoft Excel', 2.1, 75),
      app('Logic Pro', 2.0, 300), app('Blender', 1.6, 160), app('Google Chrome', 1.4, 1), app('Arc', 0.9, 30), app('Cursor', 0.72, 2), app('Visual Studio Code', 0.62, 9),
      app('Figma', 0.58, 4), app('Slack', 0.52, 1), app('Discord', 0.48, 14), app('Claude', 0.46, 0), app('Spotify', 0.38, 1), app('OrbStack', 0.42, 1),
      app('T3 Code', 0.31, 0), app('Notion', 0.36, 22), app('ChatGPT', 0.29, 6), app('Steam', 0.21, 400), app('Zoom', 0.33, 180),
    ]),
    D('Library', [
      D('Developer', [D('CoreSimulator', [D('Volumes', [bulk('iOS_23A343', 8.4 * GB, 4, undefined, 200, 'dmg'), bulk('iOS_24A5260', 9.1 * GB, 4, undefined, 20, 'dmg'), bulk('watchOS_11', 4.2 * GB, 4, 'extras', 300, 'dmg')])]), bulk('CommandLineTools', 2.4 * GB, 10, undefined, 60)]),
      D('Application Support', [bulk('Adobe', 2.6 * GB, 14, undefined, 90), bulk('Apple', 0.8 * GB, 8, undefined, 30)]),
      bulk('Caches', 1.1 * GB, 14, 'cache', 2),
      bulk('Updates', 2.8 * GB, 3, 'installers', 140, 'pkg'),
      D('Logs', [bulk('DiagnosticReports', 0.4 * GB, 30, 'logs', 20, 'ips')]),
    ]),
    D('opt', [D('homebrew', [bulk('Cellar', 4.1 * GB, 40, undefined, 30), bulk('Caskroom', 0.9 * GB, 6, undefined, 30), bulk('lib', 0.6 * GB, 20, undefined, 30)])]),
    D('private', [D('var', [bulk('folders', 3.6 * GB, 30, 'cache', 1), D('log', [bulk('asl', 0.3 * GB, 20, 'logs', 10, 'log')]), bulk('db', 0.9 * GB, 10, undefined, 2)])]),
    D('Users', [D('alex', [
      D('Library', [
        D('Developer', [
          D('Xcode', [
            D('DerivedData', ['Disko-bxkqaz', 'MealClub-fhdwuy', 'Atlas-ahzqkx', 'Wink-cgxzmm', 'Horizon-dkkqzt', 'ModuleCache.noindex'].map((p, i) => bulk(p, [7.2, 9.4, 3.1, 2.4, 1.6, 0.9][i] * GB, 12, undefined, [1, 3, 18, 40, 70, 3][i])), 'build', 1),
            D('iOS DeviceSupport', [bulk('iPhone16,2 18.4 (22E240)', 3.4 * GB, 6, undefined, 240), bulk('iPhone16,2 18.6.2 (22G100)', 3.3 * GB, 6, undefined, 120), bulk('iPhone17,1 26.1 (23B85)', 3.6 * GB, 6, undefined, 8)], 'developer'),
            D('Archives', [bulk('2026-03-11', 1.1 * GB, 3, undefined, 240, 'xcarchive'), bulk('2026-06-02', 1.2 * GB, 3, undefined, 160, 'xcarchive'), bulk('2026-10-24', 1.3 * GB, 3, undefined, 23, 'xcarchive')], 'developer'),
          ]),
          D('CoreSimulator', [D('Devices', Array.from({ length: 9 }, (_, i) => bulk(uuid().toUpperCase(), (0.6 + rnd() * 2.6) * GB, 8, undefined, 10 + i * 30)), 'developer'), bulk('Caches', 2.1 * GB, 10, 'cache', 4)]),
        ]),
        D('Caches', [bulk('com.apple.dt.Xcode', 1.4 * GB, 10, 'cache', 1), bulk('Google', 2.1 * GB, 10, 'cache', 0), bulk('com.spotify.client', 1.8 * GB, 10, 'cache', 1), bulk('Homebrew', 3.2 * GB, 14, 'packages', 6, 'tar.gz'), bulk('pip', 0.6 * GB, 10, 'packages', 50), bulk('Yarn', 1.1 * GB, 12, 'packages', 90), bulk('ms-playwright', 1.6 * GB, 6, 'cache', 30), bulk('com.figma.Desktop', 0.7 * GB, 6, 'cache', 4), bulk('JetBrains', 1.3 * GB, 8, 'leftovers', 380)]),
        D('Application Support', [
          D('Adobe', [D('Common', [bulk('Media Cache Files', 6.3 * GB, 30, 'cache', 60, 'cfa'), bulk('Peak Files', 1.4 * GB, 20, 'cache', 60, 'pek')]), bulk('After Effects', 0.6 * GB, 8, undefined, 120)]),
          D('Google', [bulk('Chrome', 3.2 * GB, 20, undefined, 0)]), bulk('Slack', 1.1 * GB, 10, undefined, 1), bulk('discord', 0.9 * GB, 10, undefined, 14),
          bulk('Code', 1.4 * GB, 10, undefined, 9), bulk('Cursor', 2.2 * GB, 10, undefined, 2), D('Claude', [bulk('vm_bundles', 8.1 * GB, 4, undefined, 0, 'img'), bulk('Cache', 0.5 * GB, 6, 'cache', 0)]),
          D('MobileSync', [bulk('Backup', 18.4 * GB, 40, undefined, 160)]), bulk('Steam', 0.8 * GB, 8, undefined, 400),
          bulk('Sketch', 1.4 * GB, 10, 'leftovers', 520), bulk('Zoom.us', 0.3 * GB, 6, undefined, 180), bulk('Spotify', 0.5 * GB, 6, undefined, 1), bulk('Figma', 0.4 * GB, 6, undefined, 4),
          bulk('com.parallels.desktop', 0.9 * GB, 6, 'leftovers', 610),
        ]),
        D('Containers', [D('com.docker.docker', [D('Data', [D('vms', [F('Docker.raw', 23.8 * GB, 340, 'containers')])])], 'containers', 340), bulk('com.apple.mail', 3.4 * GB, 30, undefined, 0), bulk('com.apple.Safari', 0.6 * GB, 10, undefined, 1)]),
        bulk('Group Containers', 4.1 * GB, 20, undefined, 3),
        bulk('Logs', 1.6 * GB, 30, 'logs', 2, 'log'),
        D('Messages', [bulk('Attachments', 6.2 * GB, 60, undefined, 2, 'heic')]),
        { n: 'Mail', err: 'Needs Full Disk Access', kids: [F('V10', 1.2 * GB, 1)] },
      ]),
      D('Documents', [
        D('Development', [
          rustProject('Disko', 8.4, 0, [nodeModules(0.6, 0)]), nodeProject('MealClub', 1.4, 1, [D('.next', [bulk('cache', 1.8 * GB, 10, undefined, 1)], 'build', 1), D('ios', [D('build', [bulk('Build', 2.6 * GB, 10, undefined, 3)], 'build', 3)])]),
          nodeProject('horizon', 0.9, 6), nodeProject('vtf3', 1.1, 2, [rustTarget(3.4, 2)]), nodeProject('old-portfolio', 0.7, 410), nodeProject('hackathon-2025', 1.2, 380),
          D('python-experiments', [D('.venv', [bulk('lib', 1.9 * GB, 20, undefined, 300)], 'build', 300), D('__pycache__', [bulk('cache', 0.1 * GB, 10)], 'build', 300), bulk('notebooks', 0.4 * GB, 10, undefined, 300, 'ipynb')], undefined, 300),
        ]),
        bulk('Invoices', 0.2 * GB, 40, undefined, 30, 'pdf'), bulk('Scans', 0.9 * GB, 30, undefined, 200, 'pdf'), bulk('Keynote', 2.1 * GB, 12, undefined, 90, 'key'),
        D('Final Cut', [F('Wedding 2025.fcpbundle', 11.2 * GB, 330, 'forgotten'), F('Trip to Lisbon.fcpbundle', 6.4 * GB, 260)]),
      ]),
      D('Development', [rustProject('atlas', 11.2, 0, [nodeModules(0.4, 0)]), nodeProject('openrive', 1.2, 21), nodeProject('rive-lab', 0.8, 21), rustProject('wink', 4.1, 9)]),
      D('Downloads', [
        F('Xcode_16.2.xip', 3.4 * GB, 70, 'installers'), F('iPhone17,1_26.1_23B85_Restore.ipsw', 7.8 * GB, 9, 'installers'), F('Docker.dmg', 0.62 * GB, 340, 'installers'),
        F('Blender-4.2-macos-arm64.dmg', 0.41 * GB, 160, 'installers'), F('Photoshop_Set-Up.dmg', 2.1 * GB, 12, 'installers'), F('GoogleChrome.dmg', 0.22 * GB, 400, 'installers'),
        F('screen-recording-final.mov', 4.2 * GB, 210, 'forgotten'), F('dataset-2025.zip', 2.8 * GB, 300, 'forgotten'), bulk('papers', 0.3 * GB, 40, undefined, 100, 'pdf'), bulk('screenshots', 0.5 * GB, 60, undefined, 20, 'png'),
      ]),
      D('Movies', [F('GoPro raw 2025.mov', 9.6 * GB, 390, 'forgotten'), bulk('Exports', 3.2 * GB, 10, undefined, 120, 'mp4')]),
      D('Music', [bulk('Music', 5.8 * GB, 200, undefined, 300, 'm4a')]),
      D('Pictures', [bulk('Photos Library.photoslibrary', 38.2 * GB, 60, undefined, 0, 'heic'), bulk('Wallpapers', 0.4 * GB, 20, undefined, 300, 'jpg')]),
      bulk('Desktop', 2.1 * GB, 30, undefined, 3, 'png'),
      D('.Trash', [F('old-render.mov', 2.4 * GB, 15), F('Untitled Folder', 0.6 * GB, 20), F('Sketch.app', 1.2 * GB, 30)], 'trash', 15),
      D('.cargo', [D('registry', [bulk('cache', 1.4 * GB, 20, undefined, 8, 'crate'), bulk('src', 1.8 * GB, 20, undefined, 8, 'rs')], 'packages', 8), bulk('bin', 0.3 * GB, 6)]),
      D('.rustup', [D('toolchains', [bulk('stable-aarch64-apple-darwin', 1.4 * GB, 10, undefined, 6), bulk('1.79.0-aarch64-apple-darwin', 1.3 * GB, 10, 'old-versions', 300), bulk('1.81.0-aarch64-apple-darwin', 1.3 * GB, 10, 'old-versions', 220), bulk('nightly-2026-05-02-aarch64-apple-darwin', 1.5 * GB, 10, 'old-versions', 190)])]),
      D('.npm', [bulk('_cacache', 3.1 * GB, 30, 'packages', 3)]),
      D('.bun', [D('install', [bulk('cache', 2.2 * GB, 30, 'packages', 0)])]),
      D('.cache', [D('huggingface', [bulk('hub', 4.2 * GB, 6, 'extras', 120, 'safetensors')]), bulk('uv', 1.3 * GB, 20, 'packages', 10)]),
      D('.ollama', [D('models', [bulk('blobs', 9.6 * GB, 5, 'extras', 150, 'gguf')])]),
      D('.claude', [D('projects', claudeProjects.map((p, i) => D(p, sessionFiles(4 + Math.floor(rnd() * 14), [26, 14, 9, 4, 3, 2, 5, 3][i], [3, 2, 40, 21, 30, 21, 60, 18][i] + 30)))), bulk('shell-snapshots', 0.05 * GB, 20, undefined, 1), D('todos', [F('todos.json', 4 * MB, 0)])]),
      D('.codex', [D('sessions', [D('2026', ['09', '10', '11'].map((m, i) => D(m, sessionFiles(6 + i * 4, 6, 60 - i * 25))))]), D('worktrees', ['a1f2', 'b7c9', 'c033', 'd4e1'].map((w, i) => D(w, [nodeProject(['MealClub', 'horizon', 'Disko', 'atlas'][i], 0.4 + i * 0.3, [12, 40, 3, 60][i])])))]),
      D('.t3', [D('worktrees', ['Disko', 'atlas', 'MealClub', 'horizon', 'vtf3'].map((p, i) => D(p, Array.from({ length: 2 + (i % 3) }, (_, k) => rustProject(`t3code-${uuid().slice(0, 8)}`, [1.6, 2.4, 0.9, 0.5, 1.1][i] * (0.5 + rnd()), 1 + k * 9 + i * 7)))), 'agent-data'), bulk('userdata', 0.4 * GB, 20, undefined, 0), D('scratch', Array.from({ length: 4 }, (_, i) => bulk(`2026-11-${10 + i}-task-${uuid().slice(0, 8)}`, (0.2 + rnd()) * GB, 6, undefined, 6 - i)))]),
      D('.orbstack', [D('data', [F('data.img', 6.2 * GB, 1, 'containers')])]),
      D('.nvm', [D('versions', [D('node', ['v18.20.4', 'v20.18.0', 'v22.11.0', 'v26.12.1'].map((v, i) => bulk(v, 0.21 * GB, 10, i < 3 ? 'old-versions' : undefined, [500, 400, 260, 10][i])))])]),
      D('.gradle', [bulk('caches', 2.1 * GB, 20, 'packages', 200)]),
    ])]),
  ]);
}

let root: MNode;
const byPath = new Map<string, MNode>();
let leafCount = 0;

function build(s: Spec, parent: MNode | null, inherited?: Category): MNode {
  const path = parent ? (parent.path === '/' ? `/${s.n}` : `${parent.path}/${s.n}`) : '/';
  const c = s.c ?? inherited;
  const n: MNode = {
    name: s.n || 'Macintosh HD', path, kind: s.kids ? 'dir' : 'file', own: s.b ?? 0, children: [], parent,
    mtime: NOW - Math.floor((s.age ?? 30) * DAY), clutter: c, error: s.err, t: 0,
  };
  byPath.set(path, n);
  if (s.kids) n.children = s.kids.map((k) => build(k, n, c));
  return n;
}

function assignTimes() {
  // A parallel-ish walk: the big top folders are entered in turn, leaves spread over the scan.
  const leaves: MNode[] = [];
  const visit = (n: MNode) => { if (n.kind === 'file') leaves.push(n); n.children.forEach(visit); };
  visit(root);
  leafCount = leaves.length;
  leaves.forEach((l, i) => { l.t = PROBE_MS + ((i + rnd() * 40) / leaves.length) * (SCAN_MS - PROBE_MS - 400); });
  const dirT = (n: MNode): number => { if (n.kind === 'dir') n.t = n.children.length ? Math.min(...n.children.map(dirT)) : PROBE_MS; return n.t; };
  dirT(root);
}

function reset() {
  seed = 7;
  byPath.clear();
  root = build(spec(), null);
  assignTimes();
}
reset();

// ---------- scan clock ----------
let scanStart = performance.now();
let scanDone = false;
const elapsed = () => (scanDone ? Infinity : performance.now() - scanStart);

const sizeOf = (n: MNode, t: number): number => (n.kind === 'file' ? (n.t <= t ? n.own : 0) : n.children.reduce((s, c) => s + sizeOf(c, t), 0));
const filesOf = (n: MNode, t: number): number => (n.kind === 'file' ? (n.t <= t ? 1 : 0) : n.children.reduce((s, c) => s + filesOf(c, t), 0));
const newestOf = (n: MNode): number => (n.kind === 'file' ? n.mtime : Math.max(n.mtime, ...n.children.map(newestOf)));

function view(n: MNode, depth: number, t: number): TreeNode {
  const out: TreeNode = { name: n.name, path: n.path, kind: n.kind, bytes: sizeOf(n, t), files: filesOf(n, t), newest: newestOf(n) };
  if (n.clutter) out.clutter = n.clutter;
  if (n.error) out.error = n.error;
  if (depth > 0 && n.kind === 'dir') {
    const kids = n.children.map((c) => ({ c, b: sizeOf(c, t) })).filter((x) => x.b > 0).sort((a, b) => b.b - a.b);
    const keepMin = out.bytes * 0.004;
    const shown = kids.filter((x, i) => i < 40 && (x.b >= keepMin || i < 8));
    const rest = kids.slice(shown.length);
    out.children = shown.map((x) => view(x.c, depth - 1, t));
    if (rest.length) {
      out.children.push({ name: `${rest.length} smaller items`, path: `${n.path === '/' ? '' : n.path}/…`, kind: 'other', bytes: rest.reduce((s, x) => s + x.b, 0), files: rest.reduce((s, x) => s + filesOf(x.c, t), 0) });
    }
  }
  return out;
}

// ---------- volume ----------
const TOTAL = 994.66 * 1e9;
const HIDDEN: HiddenSpace[] = [
  { kind: 'system', label: 'macOS', bytes: 11.8 * GB, why: 'The sealed system volume, Preboot and Recovery. Part of every Mac.' },
  { kind: 'snapshots', label: 'Time Machine snapshots', bytes: 14.6 * GB, why: 'Local snapshots macOS keeps until it needs the space.' },
  { kind: 'vm', label: 'Swap & sleep image', bytes: 4.0 * GB, why: 'Memory macOS parked on disk. It shrinks after a restart.' },
  { kind: 'unreadable', label: 'Folders Disko can’t read', bytes: 2.7 * GB, why: 'Give Disko Full Disk Access to see inside them.' },
];
let fda = new URLSearchParams(location.search).get('fda') === '1';
function volume(): VolumeInfo {
  const scanned = sizeOf(root, Infinity);
  const hidden = fda ? HIDDEN.filter((h) => h.kind !== 'unreadable') : HIDDEN;
  const used = scanned + hidden.reduce((s, h) => s + h.bytes, 0);
  return { name: 'Macintosh HD', total: TOTAL, used, free: TOTAL - used, purgeable: 9.2 * GB, scanned: sizeOf(root, elapsed()), hidden };
}

// ---------- findings ----------
interface FDef { rule: string; category: Category; title: string; detail?: string; icon: string; paths: string[]; safety: Finding['safety']; action?: Finding['action']; regrows: boolean; why: string; busy?: string; admin?: boolean; source?: Finding['source']; probe?: boolean }
const H = (p: string) => `${HOME}/${p}`;
const del = { type: 'delete' } as const;
const trash = { type: 'trash' } as const;

function findingDefs(): FDef[] {
  const dd = H('Library/Developer/Xcode/DerivedData');
  const defs: FDef[] = [
    { rule: 'build.derived-data', category: 'build', title: 'Xcode build data', detail: 'DerivedData', icon: 'xcode', paths: [dd], safety: 'safe', regrows: true, why: 'Xcode rebuilds this the next time you build.', busy: undefined, probe: true },
    { rule: 'build.rust-target', category: 'build', title: 'Disko', detail: 'Rust target', icon: 'rust', paths: [H('Documents/Development/Disko/target')], safety: 'safe', regrows: true, why: 'cargo build recreates it.', busy: 'cargo is building Disko right now' },
    { rule: 'build.rust-target', category: 'build', title: 'atlas', detail: 'Rust target', icon: 'rust', paths: [H('Development/atlas/target')], safety: 'safe', regrows: true, why: 'cargo build recreates it.' },
    { rule: 'build.rust-target', category: 'build', title: 'wink', detail: 'Rust target', icon: 'rust', paths: [H('Development/wink/target')], safety: 'safe', regrows: true, why: 'Last built 9 days ago. cargo build recreates it.' },
    { rule: 'build.rust-target', category: 'build', title: 'vtf3', detail: 'Rust target', icon: 'rust', paths: [H('Documents/Development/vtf3/target')], safety: 'safe', regrows: true, why: 'cargo build recreates it.' },
    { rule: 'build.next', category: 'build', title: 'MealClub', detail: '.next cache + iOS build', icon: 'node', paths: [H('Documents/Development/MealClub/.next'), H('Documents/Development/MealClub/ios/build')], safety: 'safe', regrows: true, why: 'Next.js and Xcode rebuild these.' },
    ...['old-portfolio', 'hackathon-2025', 'horizon', 'vtf3', 'MealClub'].map((p) => ({ rule: 'build.node-modules', category: 'build' as Category, title: p, detail: 'node_modules', icon: 'node', paths: [H(`Documents/Development/${p}/node_modules`)], safety: 'safe' as const, regrows: true, why: 'bun install brings it back in seconds.' })),
    ...['openrive', 'rive-lab', 'atlas'].map((p) => ({ rule: 'build.node-modules', category: 'build' as Category, title: p, detail: 'node_modules', icon: 'node', paths: [H(`Development/${p}/node_modules`)], safety: 'safe' as const, regrows: true, why: 'bun install brings it back in seconds.' })),
    { rule: 'build.python-venv', category: 'build', title: 'python-experiments', detail: '.venv + __pycache__', icon: 'python', paths: [H('Documents/Development/python-experiments/.venv'), H('Documents/Development/python-experiments/__pycache__')], safety: 'safe', regrows: true, why: 'Untouched for 10 months. uv sync recreates it.' },
    { rule: 'cache.user', category: 'cache', title: 'App caches', detail: 'Chrome, Spotify, Figma, Xcode', icon: 'cache', paths: ['Google', 'com.spotify.client', 'com.figma.Desktop', 'com.apple.dt.Xcode'].map((c) => H(`Library/Caches/${c}`)), safety: 'safe', regrows: true, why: 'Apps refill these as they need them.', probe: true },
    { rule: 'cache.adobe-media', category: 'cache', title: 'Adobe media cache', detail: 'Premiere & After Effects', icon: 'app:/Applications/Adobe Premiere Pro 2026.app', paths: [H('Library/Application Support/Adobe/Common/Media Cache Files'), H('Library/Application Support/Adobe/Common/Peak Files')], safety: 'safe', regrows: true, why: 'Premiere re-conforms media when a project opens.' },
    { rule: 'cache.playwright', category: 'cache', title: 'Playwright browsers', icon: 'cache', paths: [H('Library/Caches/ms-playwright')], safety: 'safe', regrows: true, why: 'Downloaded again by playwright install.' },
    { rule: 'cache.simulator', category: 'cache', title: 'Simulator caches', icon: 'xcode', paths: [H('Library/Developer/CoreSimulator/Caches')], safety: 'safe', regrows: true, why: 'Simulator rebuilds its dyld cache.' },
    { rule: 'cache.system-temp', category: 'cache', title: 'System temporary files', icon: 'cache', paths: ['/private/var/folders'], safety: 'safe', regrows: true, why: 'Temporary items of apps that are not running.', admin: true },
    { rule: 'packages.homebrew', category: 'packages', title: 'Homebrew downloads', icon: 'brew', paths: [H('Library/Caches/Homebrew')], safety: 'safe', action: { type: 'command', argv: ['brew', 'cleanup', '--prune=all'], label: 'brew cleanup' }, regrows: true, why: 'Bottles already installed.', probe: true },
    { rule: 'packages.npm', category: 'packages', title: 'npm cache', icon: 'node', paths: [H('.npm/_cacache')], safety: 'safe', regrows: true, why: 'npm downloads packages again on demand.' },
    { rule: 'packages.bun', category: 'packages', title: 'Bun cache', icon: 'bun', paths: [H('.bun/install/cache')], safety: 'safe', regrows: true, why: 'bun install refills it.' },
    { rule: 'packages.cargo', category: 'packages', title: 'Cargo registry', icon: 'rust', paths: [H('.cargo/registry')], safety: 'safe', regrows: true, why: 'Crates download again when you build.' },
    { rule: 'packages.yarn', category: 'packages', title: 'Yarn & pip caches', icon: 'box', paths: [H('Library/Caches/Yarn'), H('Library/Caches/pip')], safety: 'safe', regrows: true, why: 'Package downloads, fetched again on demand.' },
    { rule: 'packages.uv', category: 'packages', title: 'uv cache', icon: 'python', paths: [H('.cache/uv')], safety: 'safe', action: { type: 'command', argv: ['uv', 'cache', 'clean'], label: 'uv cache clean' }, regrows: true, why: 'uv downloads wheels again on demand.' },
    { rule: 'packages.gradle', category: 'packages', title: 'Gradle caches', icon: 'box', paths: [H('.gradle/caches')], safety: 'safe', regrows: true, why: 'Last used 7 months ago.' },
    { rule: 'developer.device-support', category: 'developer', title: 'iPhone 15 Pro · iOS 18', detail: '2 old versions', icon: 'iphone', paths: [H('Library/Developer/Xcode/iOS DeviceSupport/iPhone16,2 18.4 (22E240)'), H('Library/Developer/Xcode/iOS DeviceSupport/iPhone16,2 18.6.2 (22G100)')], safety: 'safe', regrows: true, why: 'Your iPhone runs iOS 26.1 now. Xcode copies symbols again if you plug in an older one.', probe: true },
    { rule: 'developer.simulators', category: 'developer', title: 'Unused simulators', detail: '9 devices', icon: 'xcode', paths: [H('Library/Developer/CoreSimulator/Devices')], safety: 'safe', action: { type: 'command', argv: ['xcrun', 'simctl', 'delete', 'unavailable'], label: 'simctl delete' }, regrows: false, why: 'Devices for runtimes that are no longer installed.', probe: true },
    { rule: 'developer.archives', category: 'developer', title: 'Xcode archives', detail: '3 builds', icon: 'xcode', paths: [H('Library/Developer/Xcode/Archives')], safety: 'review', action: trash, regrows: false, why: 'Needed only to symbolicate crashes of shipped builds.' },
    { rule: 'developer.runtime', category: 'developer', title: 'iOS 25 Simulator', detail: 'Older runtime', icon: 'xcode', paths: ['/Library/Developer/CoreSimulator/Volumes/iOS_23A343'], safety: 'review', action: { type: 'command', argv: ['xcrun', 'simctl', 'runtime', 'delete', '23A343'], label: 'simctl runtime delete' }, regrows: false, why: 'iOS 26 runtime is installed too.', admin: true },
    { rule: 'leftovers.app-support', category: 'leftovers', title: 'Sketch', detail: 'App removed 1 year ago', icon: 'app:/Applications/Sketch.app', paths: [H('Library/Application Support/Sketch')], safety: 'review', action: trash, regrows: false, why: 'No installed app uses com.bohemiancoding.sketch3.' },
    { rule: 'leftovers.app-support', category: 'leftovers', title: 'Parallels Desktop', detail: 'App removed', icon: 'app:/Applications/Parallels Desktop.app', paths: [H('Library/Application Support/com.parallels.desktop')], safety: 'review', action: trash, regrows: false, why: 'Parallels is not installed anymore.' },
    { rule: 'leftovers.caches', category: 'leftovers', title: 'JetBrains', detail: 'IDE caches, no IDE installed', icon: 'box', paths: [H('Library/Caches/JetBrains')], safety: 'safe', regrows: false, why: 'No JetBrains IDE is installed.' },
    { rule: 'leftovers.docker-desktop', category: 'containers', title: 'Docker Desktop disk', detail: 'Docker.raw', icon: 'docker', paths: [H('Library/Containers/com.docker.docker')], safety: 'review', action: trash, regrows: false, why: 'Docker Desktop is gone; OrbStack has its own disk.', probe: true },
    { rule: 'containers.orbstack', category: 'containers', title: 'OrbStack', detail: 'Linux machines + images', icon: 'docker', paths: [H('.orbstack/data')], safety: 'review', action: { type: 'command', argv: ['orb', 'prune'], label: 'orb prune' }, regrows: false, why: 'Used yesterday. Prune unused images inside OrbStack instead.', busy: 'OrbStack is running' },
    { rule: 'old-versions.rustup', category: 'old-versions', title: 'Rust toolchains', detail: '3 older versions', icon: 'rust', paths: ['1.79.0', '1.81.0', 'nightly-2026-05-02'].map((v) => H(`.rustup/toolchains/${v}-aarch64-apple-darwin`)), safety: 'safe', action: { type: 'command', argv: ['rustup', 'toolchain', 'uninstall', '1.79.0', '1.81.0', 'nightly-2026-05-02'], label: 'rustup uninstall' }, regrows: false, why: 'stable 1.92 is your default; no project pins these.' },
    { rule: 'old-versions.nvm', category: 'old-versions', title: 'Node.js versions', detail: 'v18, v20, v22', icon: 'node', paths: ['v18.20.4', 'v20.18.0', 'v22.11.0'].map((v) => H(`.nvm/versions/node/${v}`)), safety: 'review', action: trash, regrows: false, why: 'v26 is the default; no .nvmrc asks for these.' },
    { rule: 'installers.xip', category: 'installers', title: 'Xcode 16.2 installer', icon: 'xcode', paths: [H('Downloads/Xcode_16.2.xip')], safety: 'safe', regrows: false, why: 'Xcode 26.1 is installed.', probe: true },
    { rule: 'installers.ipsw', category: 'installers', title: 'iPhone restore image', detail: 'iOS 26.1', icon: 'iphone', paths: [H('Downloads/iPhone17,1_26.1_23B85_Restore.ipsw')], safety: 'safe', regrows: false, why: 'Finder downloads it again if needed.' },
    { rule: 'installers.dmg', category: 'installers', title: 'Disk images', detail: '4 installed apps', icon: 'disc', paths: ['Docker.dmg', 'Blender-4.2-macos-arm64.dmg', 'Photoshop_Set-Up.dmg', 'GoogleChrome.dmg'].map((f) => H(`Downloads/${f}`)), safety: 'safe', regrows: false, why: 'Each app is already installed from it.' },
    { rule: 'installers.macos-updates', category: 'installers', title: 'Leftover macOS updates', icon: 'apple', paths: ['/Library/Updates'], safety: 'safe', regrows: false, why: 'Already applied updates.', admin: true },
    { rule: 'logs.user', category: 'logs', title: 'Logs & crash reports', icon: 'log', paths: [H('Library/Logs'), '/Library/Logs/DiagnosticReports', '/private/var/log/asl'], safety: 'safe', regrows: true, why: 'Old diagnostics nobody reads.' },
    { rule: 'agent.t3-worktrees', category: 'agent-data', title: 'T3 Code worktrees', detail: 'Finished threads', icon: 't3', paths: [H('.t3/worktrees')], safety: 'review', action: trash, regrows: false, why: 'Worktrees of threads that are merged or closed.' },
    { rule: 'agent.codex-worktrees', category: 'agent-data', title: 'Codex worktrees', icon: 'codex', paths: [H('.codex/worktrees')], safety: 'review', action: trash, regrows: false, why: 'Tasks finished more than 10 days ago.' },
    { rule: 'extras.ollama', category: 'extras', title: 'Ollama models', detail: '5 models', icon: 'sparkles', paths: [H('.ollama/models')], safety: 'review', action: trash, regrows: false, why: 'Not run for 5 months.' },
    { rule: 'extras.huggingface', category: 'extras', title: 'Hugging Face models', icon: 'sparkles', paths: [H('.cache/huggingface')], safety: 'review', action: trash, regrows: false, why: 'Downloaded models, fetched again on demand.' },
    { rule: 'extras.watchos-runtime', category: 'extras', title: 'watchOS Simulator', icon: 'watch', paths: ['/Library/Developer/CoreSimulator/Volumes/watchOS_11'], safety: 'review', action: trash, regrows: false, why: 'No watchOS project on this Mac.', admin: true },
    { rule: 'forgotten.big-file', category: 'forgotten', title: 'GoPro raw 2025.mov', icon: 'film', paths: [H('Movies/GoPro raw 2025.mov')], safety: 'review', action: trash, regrows: false, why: 'Not opened in 13 months.' },
    { rule: 'forgotten.big-file', category: 'forgotten', title: 'Wedding 2025', detail: 'Final Cut library', icon: 'app:/Applications/Final Cut Pro.app', paths: [H('Documents/Final Cut/Wedding 2025.fcpbundle')], safety: 'review', action: trash, regrows: false, why: 'Not opened in 11 months.' },
    { rule: 'forgotten.big-file', category: 'forgotten', title: 'screen-recording-final.mov', icon: 'film', paths: [H('Downloads/screen-recording-final.mov')], safety: 'review', action: trash, regrows: false, why: 'In Downloads for 7 months.' },
    { rule: 'forgotten.big-file', category: 'forgotten', title: 'dataset-2025.zip', icon: 'archive', paths: [H('Downloads/dataset-2025.zip')], safety: 'review', action: trash, regrows: false, why: 'In Downloads for 10 months.' },
    { rule: 'trash.user', category: 'trash', title: 'Trash', icon: 'trash', paths: [H('.Trash')], safety: 'review', action: del, regrows: false, why: 'Already thrown away. Emptying it is permanent.', probe: true },
  ];
  return defs;
}

function fid(rule: string, paths: string[]) {
  let h = 0x811c9dc5;
  for (const ch of rule + [...paths].sort().join('\0')) { h ^= ch.charCodeAt(0); h = Math.imul(h, 0x01000193); }
  return `${rule}:${(h >>> 0).toString(16).padStart(8, '0')}`;
}

let defs: FDef[] = [];
const emitted = new Map<string, Finding>();

function findingNow(d: FDef, t: number): Finding | null {
  const nodes = d.paths.map((p) => byPath.get(p)).filter(Boolean) as MNode[];
  if (!nodes.length) return null;
  const bytes = nodes.reduce((s, n) => s + sizeOf(n, d.probe ? Infinity : t), 0);
  if (bytes <= 0) return null;
  return {
    id: fid(d.rule, d.paths), rule: d.rule, category: d.category, title: d.title, detail: d.detail, icon: d.icon,
    paths: nodes.map((n) => n.path), bytes, last_used: Math.max(...nodes.map(newestOf)), safety: d.safety,
    action: d.action ?? (d.safety === 'safe' ? del : trash), regrows: d.regrows, why: d.why, busy: d.busy, admin: d.admin, source: d.source ?? 'builtin',
  };
}

function readyAt(d: FDef) {
  if (d.probe) return 200 + (d.rule.length * 37) % PROBE_MS;
  const nodes = d.paths.map((p) => byPath.get(p)).filter(Boolean) as MNode[];
  const maxT = (n: MNode): number => (n.kind === 'file' ? n.t : Math.max(n.t, ...n.children.map(maxT)));
  return Math.max(0, ...nodes.map(maxT));
}

// ---------- events ----------
type Cb = (p: unknown) => void;
const listeners = new Map<string, Set<Cb>>();
function emit<K extends keyof Events>(event: K, payload: Events[K]) {
  listeners.get(event)?.forEach((cb) => { try { cb(payload); } catch (e) { console.error(e); } });
}

let timers: number[] = [];
function startScan() {
  timers.forEach((t) => clearInterval(t));
  timers = [];
  scanStart = performance.now();
  scanDone = false;
  emitted.clear();
  defs = findingDefs();
  const pending = new Set(defs.map((_, i) => i));
  const tickProgress = () => {
    const t = elapsed();
    const bytes = sizeOf(root, t);
    const done = t >= SCAN_MS;
    const files = filesOf(root, t);
    const p: ScanProgress = {
      phase: done ? 'done' : t < PROBE_MS ? 'probes' : 'walk', files: files * 412 + Math.floor(rnd() * 400), dirs: Math.floor(files * 61), bytes,
      elapsed_ms: Math.min(Math.floor(t), SCAN_MS), current: done ? '' : currentFolder(t), fraction: Math.min(1, bytes / sizeOf(root, Infinity)),
    };
    emit('scan:progress', p);
    // Findings that are ready now.
    const batch: Finding[] = [];
    for (const i of [...pending]) {
      if (readyAt(defs[i]) <= t) {
        pending.delete(i);
        const f = findingNow(defs[i], t);
        if (f) { emitted.set(f.id, f); batch.push(f); }
      }
    }
    if (batch.length) emit('clutter:upsert', batch);
    if (done) {
      scanDone = true;
      timers.forEach((x) => clearInterval(x));
      timers = [];
      emit('scan:tree', view(root, 3, Infinity));
      emit('volume', volume());
    }
  };
  timers.push(window.setInterval(tickProgress, 250));
  timers.push(window.setInterval(() => { if (!scanDone) { emit('scan:tree', view(root, 3, elapsed())); emit('volume', volume()); } }, 1000));
}

function currentFolder(t: number) {
  let best: MNode = root;
  const visit = (n: MNode) => { if (n.kind === 'dir' && n.t <= t && n.t >= best.t) best = n; n.children.forEach(visit); };
  visit(root);
  return best.path;
}

// ---------- removal ----------
const trashed = new Map<string, { node: MNode; from: string }>();
const trashDir = () => byPath.get(H('.Trash'))!;

function detach(n: MNode) {
  const p = n.parent;
  if (!p) return;
  p.children = p.children.filter((c) => c !== n);
  const drop = (x: MNode) => { byPath.delete(x.path); x.children.forEach(drop); };
  drop(n);
}
function reparent(n: MNode, parent: MNode, name: string) {
  n.parent = parent;
  n.name = name;
  const fix = (x: MNode, path: string) => { x.path = path; byPath.set(path, x); x.children.forEach((c) => fix(c, `${path}/${c.name}`)); };
  fix(n, `${parent.path}/${name}`);
  parent.children.push(n);
}

function removePaths(paths: string[], mode: 'delete' | 'trash', res: CleanResult) {
  for (const p of paths) {
    const n = byPath.get(p);
    if (!n) { res.failed.push({ path: p, error: 'Not found' }); continue; }
    const bytes = sizeOf(n, Infinity);
    const inTrash = p.startsWith(H('.Trash'));
    detach(n);
    if (mode === 'trash' && !inTrash) {
      let name = n.name;
      while (byPath.has(`${trashDir().path}/${name}`)) name = `${name} copy`;
      reparent(n, trashDir(), name);
      trashed.set(n.path, { node: n, from: p });
      res.trashed.push({ from: p, to: n.path, bytes });
    } else if (p === H('.Trash')) {
      // Emptying the Trash keeps the folder itself.
      n.children = [];
      reparent(n, byPath.get(HOME)!, '.Trash');
      res.freed += bytes;
    } else res.freed += bytes;
  }
}

function changed(paths: string[]) {
  const dirs = new Set<string>();
  for (const p of paths) {
    let cur = p.slice(0, p.lastIndexOf('/')) || '/';
    dirs.add(cur);
    while (cur !== '/') { cur = cur.slice(0, cur.lastIndexOf('/')) || '/'; dirs.add(cur); }
  }
  // Refresh affected findings.
  const upsert: Finding[] = [];
  const gone: string[] = [];
  for (const d of defs) {
    const id = fid(d.rule, d.paths);
    if (!emitted.has(id)) continue;
    const f = findingNow(d, Infinity);
    if (!f) { emitted.delete(id); gone.push(id); }
    else if (f.bytes !== emitted.get(id)!.bytes) { emitted.set(id, f); upsert.push(f); }
  }
  setTimeout(() => {
    emit('tree:changed', { paths: [...dirs] });
    if (gone.length) emit('clutter:remove', gone);
    if (upsert.length) emit('clutter:upsert', upsert);
    emit('volume', volume());
  }, 40);
}

function clean(ids: string[], paths: string[]): CleanResult {
  const res: CleanResult = { freed: 0, removed: [], trashed: [], failed: [], skipped: [] };
  const touched: string[] = [];
  for (const id of ids) {
    const f = emitted.get(id);
    if (!f) continue;
    if (f.busy) { f.paths.forEach((p) => res.skipped.push({ path: p, error: f.busy! })); continue; }
    if (memory.keep.some((k) => f.paths.some((p) => p.startsWith(k.pattern.replace('~', HOME))))) { f.paths.forEach((p) => res.skipped.push({ path: p, error: 'On your keep list' })); continue; }
    removePaths(f.paths, f.action.type === 'trash' ? 'trash' : 'delete', res);
    touched.push(...f.paths);
    res.removed.push(id);
  }
  removePaths(paths, 'trash', res);
  touched.push(...paths);
  changed([...touched, H('.Trash/x')]);
  return res;
}

function undo(items: Trashed[]) {
  let restored = 0;
  const failed: { path: string; error: string }[] = [];
  for (const t of items) {
    const e = trashed.get(t.to);
    const parentPath = t.from.slice(0, t.from.lastIndexOf('/')) || '/';
    const parent = byPath.get(parentPath);
    if (!e || !parent) { failed.push({ path: t.from, error: 'No longer in the Trash' }); continue; }
    detach(e.node);
    reparent(e.node, parent, t.from.slice(parentPath.length + 1));
    trashed.delete(t.to);
    restored++;
  }
  // Findings whose paths came back reappear.
  const back: Finding[] = [];
  for (const d of defs) {
    const id = fid(d.rule, d.paths);
    if (emitted.has(id)) continue;
    if (!items.some((t) => d.paths.some((p) => p === t.from || t.from.startsWith(p + '/')))) continue;
    const f = findingNow(d, Infinity);
    if (f) { emitted.set(id, f); back.push(f); }
  }
  changed([...items.map((t) => t.from), H('.Trash/x')]);
  if (back.length) setTimeout(() => emit('clutter:upsert', back), 60);
  return { restored, failed };
}

// ---------- apps ----------
const APP_DATA: Record<string, string[]> = {
  'Adobe Premiere Pro 2026': [H('Library/Application Support/Adobe/Common')],
  'Adobe After Effects 2026': [H('Library/Application Support/Adobe/After Effects')],
  'Google Chrome': [H('Library/Application Support/Google/Chrome'), H('Library/Caches/Google')],
  Slack: [H('Library/Application Support/Slack')], Discord: [H('Library/Application Support/discord')],
  'Visual Studio Code': [H('Library/Application Support/Code')], Cursor: [H('Library/Application Support/Cursor')],
  Claude: [H('Library/Application Support/Claude')], Spotify: [H('Library/Application Support/Spotify'), H('Library/Caches/com.spotify.client')],
  Steam: [H('Library/Application Support/Steam')], Zoom: [H('Library/Application Support/Zoom.us')], Figma: [H('Library/Application Support/Figma'), H('Library/Caches/com.figma.Desktop')],
  OrbStack: [H('.orbstack')], Xcode: [H('Library/Developer/Xcode'), H('Library/Caches/com.apple.dt.Xcode')],
};
const APPLE = new Set(['Xcode', 'Final Cut Pro', 'Logic Pro']);
function apps(): AppInfo[] {
  const dir = byPath.get('/Applications');
  if (!dir) return [];
  return dir.children.filter((c) => c.name.endsWith('.app')).map((c) => {
    const name = c.name.replace(/\.app$/, '');
    const data = (APP_DATA[name] ?? []).filter((p) => byPath.has(p));
    const used = NOW - c.mtime < DAY ? NOW - 3600 * (1 + (name.length % 9)) : c.mtime;
    return {
      name, bundle_id: `com.example.${name.toLowerCase().replace(/\W/g, '')}`, path: c.path, version: `${(name.length % 7) + 1}.${name.length % 4}`,
      icon: `app:${c.path}`, app_bytes: sizeOf(c, Infinity), data_bytes: data.reduce((s, p) => s + sizeOf(byPath.get(p)!, Infinity), 0), data_paths: data,
      last_used: used, running: ['Google Chrome', 'Slack', 'Claude', 'T3 Code', 'Spotify', 'OrbStack', 'Cursor'].includes(name), apple: APPLE.has(name),
    };
  });
}

// ---------- agent sessions ----------
const TITLES = ['Fix the window drag region', 'Rebuild the scan engine', 'Add sunburst hover', 'Migrate keel to Atlas', 'Telegram mini app', 'Recipe embeddings job', 'Space Lens bubbles', 'Why is the build slow?', 'Release 0.6.1', 'Write the contract', 'Investigate SIGBUS in SQLite', 'Refactor clutter rules', 'Ad-hoc phone builds', 'Rive loader animation', 'Dark mode polish', 'Map zoom transitions', 'Fix flaky tests', 'Onboarding copy', 'Tailscale watchdog', 'Undo for Trash'];
function sessions(): AgentTool[] {
  const tools: AgentTool[] = [];
  const projDir = byPath.get(H('.claude/projects'));
  if (projDir) {
    const projects: AgentProject[] = projDir.children.map((p) => {
      const ss: AgentSession[] = p.children.map((f, i) => ({ id: f.name.replace('.jsonl', ''), title: TITLES[(i * 7 + p.name.length) % TITLES.length], bytes: f.own, started: f.mtime - 3600 * 5, last_used: f.mtime, paths: [f.path] }));
      return { name: p.name.split('-').filter(Boolean).pop() === 'alex' ? 'Home folder' : p.name.split('-').pop()!, path: p.name.replace(/-/g, '/'), bytes: sizeOf(p, Infinity), last_used: newestOf(p), sessions: ss.sort((a, b) => (b.last_used ?? 0) - (a.last_used ?? 0)) };
    }).filter((p) => p.bytes > 0);
    tools.push({ tool: 'claude-code', name: 'Claude Code', icon: 'claude', bytes: projects.reduce((s, p) => s + p.bytes, 0), projects });
  }
  const t3 = byPath.get(H('.t3/worktrees'));
  if (t3) {
    const projects: AgentProject[] = t3.children.map((p) => ({
      name: p.name, path: H(`Documents/Development/${p.name}`), bytes: sizeOf(p, Infinity), last_used: newestOf(p),
      sessions: p.children.map((w, i) => ({ id: w.name, title: TITLES[(i * 5 + p.name.length * 3) % TITLES.length], bytes: sizeOf(w, Infinity), started: newestOf(w) - DAY, last_used: newestOf(w), paths: [w.path] })),
    })).filter((p) => p.bytes > 0);
    tools.push({ tool: 't3code', name: 'T3 Code', icon: 't3', bytes: projects.reduce((s, p) => s + p.bytes, 0), projects });
  }
  const cx = byPath.get(H('.codex/sessions/2026'));
  const cw = byPath.get(H('.codex/worktrees'));
  if (cx || cw) {
    const projects: AgentProject[] = [];
    if (cw) for (const w of cw.children) {
      const proj = w.children[0];
      if (proj) projects.push({ name: proj.name, path: H(`Documents/Development/${proj.name}`), bytes: sizeOf(w, Infinity), last_used: newestOf(w), sessions: [{ id: w.name, title: TITLES[w.name.charCodeAt(0) % TITLES.length], bytes: sizeOf(w, Infinity), started: newestOf(w) - DAY, last_used: newestOf(w), paths: [w.path] }] });
    }
    if (cx) {
      const files = cx.children.flatMap((m) => m.children);
      projects.push({ name: 'Conversations', bytes: sizeOf(cx, Infinity), last_used: newestOf(cx), sessions: files.map((f, i) => ({ id: f.name, title: TITLES[(i * 3) % TITLES.length], bytes: f.own, started: f.mtime - 7200, last_used: f.mtime, paths: [f.path] })) });
    }
    tools.push({ tool: 'codex', name: 'Codex', icon: 'codex', bytes: projects.reduce((s, p) => s + p.bytes, 0), projects: projects.filter((p) => p.bytes > 0) });
  }
  return tools.sort((a, b) => b.bytes - a.bytes);
}

// ---------- memory ----------
let memory: Memory = {
  keep: [
    { id: 'k1', pattern: '~/Library/Application Support/MobileSync/Backup', note: 'iPhone backups', source: 'user', added: NOW - 40 * DAY },
    { id: 'k2', pattern: '~/.ollama/models/blobs/llama-3.2', source: 'agent', added: NOW - 3 * DAY },
  ],
  rules: [
    { id: 'r1', pattern: '~/Library/Caches/com.figma.Desktop', category: 'cache', title: 'Figma cache', why: 'Figma refills it.', regrows: true, safety: 'safe', source: 'user', added: NOW - 12 * DAY, shared: true },
    { id: 'r2', pattern: '~/.t3/scratch/*', category: 'agent-data', title: 'T3 scratch folders', why: 'Per-task scratch space, safe after the task ends.', regrows: false, safety: 'review', source: 'agent', added: NOW - 2 * DAY, shared: false },
    { id: 'r3', pattern: '~/Library/Caches/ms-playwright', category: 'cache', title: 'Playwright browsers', why: 'playwright install downloads them again.', regrows: true, safety: 'safe', source: 'community', added: NOW - 30 * DAY, shared: true },
    { id: 'r4', pattern: '~/Library/Application Support/Adobe/Common/Media Cache Files', category: 'cache', title: 'Adobe media cache', why: 'Premiere re-conforms media on open.', regrows: true, safety: 'safe', source: 'community', added: NOW - 60 * DAY, shared: true },
  ],
  routines: [
    { id: 'rt-build', title: 'Build artifacts', select: { categories: ['build'] }, every_days: 2, min_age_days: 2, enabled: true, last_run: NOW - DAY, last_freed: 14.2 * GB },
    { id: 'rt-logs', title: 'Logs', select: { categories: ['logs'] }, every_days: 7, min_age_days: 7, enabled: true, last_run: NOW - 4 * DAY, last_freed: 0.9 * GB },
  ],
  community: { enabled: true, rules: 412, updated: NOW - 2 * DAY },
};
function memoryUpdate(op: MemoryOp): Memory {
  const m = structuredClone(memory);
  const id = () => Math.random().toString(36).slice(2, 9);
  switch (op.type) {
    case 'keep': m.keep.unshift({ id: id(), pattern: op.pattern, note: op.note, source: 'user', added: NOW }); break;
    case 'unkeep': m.keep = m.keep.filter((k) => k.id !== op.id); break;
    case 'add-rule': m.rules.unshift(op.rule); break;
    case 'remove-rule': m.rules = m.rules.filter((r) => r.id !== op.id); break;
    case 'set-routine': { const i = m.routines.findIndex((r) => r.id === op.routine.id); if (i >= 0) m.routines[i] = op.routine; else m.routines.push(op.routine); break; }
    case 'remove-routine': m.routines = m.routines.filter((r) => r.id !== op.id); break;
    case 'community': m.community.enabled = op.enabled; break;
  }
  memory = m;
  setTimeout(() => emit('memory', memory), 10);
  return memory;
}

let mcp: McpClient[] = [
  { id: 'claude-code', name: 'Claude Code', present: true, connected: true },
  { id: 'codex', name: 'Codex', present: true, connected: false },
  { id: 'cursor', name: 'Cursor', present: true, connected: false },
];

// ---------- mock app icons ----------
const ICONS: Record<string, [string, string, string, string?]> = {
  Xcode: ['#5fb0ff', '#1458e0', '⚒'], 'Final Cut Pro': ['#3a3a40', '#111114', 'FC', '#f3c13a'], 'Adobe Premiere Pro 2026': ['#1b0a52', '#08002b', 'Pr', '#9999ff'],
  'Adobe Premiere Pro': ['#1b0a52', '#08002b', 'Pr', '#9999ff'], 'Adobe After Effects 2026': ['#1b0a52', '#08002b', 'Ae', '#d291ff'], 'Adobe Photoshop 2026': ['#062a49', '#001e36', 'Ps', '#31a8ff'],
  'Adobe Illustrator 2026': ['#3a1100', '#260a00', 'Ai', '#ff9a00'], 'Microsoft Word': ['#3b8cf0', '#185abd', 'W'], 'Microsoft Excel': ['#2bb673', '#107c41', 'X'],
  'Logic Pro': ['#4a4a52', '#1d1d22', '♪', '#9ad7ff'], Blender: ['#ff9c3b', '#e8700f', 'B'], 'Google Chrome': ['#ffffff', '#e9eef5', '◉', '#1a73e8'], Arc: ['#ff6b8b', '#6a5cff', 'A'],
  Cursor: ['#2a2a2e', '#0b0b0d', '▲', '#e8e8ea'], 'Visual Studio Code': ['#3fa9f5', '#0065a9', '⟨⟩'], Figma: ['#2c2c2c', '#1e1e1e', 'F', '#a259ff'], Slack: ['#ffffff', '#f1edf3', '#', '#4a154b'],
  Discord: ['#7984f5', '#5865f2', '◕'], Claude: ['#e98b63', '#d97757', '✳'], Spotify: ['#1ed760', '#169c46', '≋', '#0b0b0b'], OrbStack: ['#3b3d8f', '#1a1b4b', '◐'],
  'T3 Code': ['#1a1a1a', '#000000', 'T3', '#f0f0f0'], Notion: ['#ffffff', '#f2f2f2', 'N', '#111'], ChatGPT: ['#ffffff', '#f2f2f2', '✺', '#111'], Steam: ['#1b2838', '#0e1621', 'S', '#c7d5e0'],
  Zoom: ['#4a8cff', '#0b5cff', '▶'], Sketch: ['#ffd25e', '#f7a600', '◆'], 'Parallels Desktop': ['#ff5a4f', '#c9241a', '∥'],
};
function mockIcon(key: string): string | null {
  if (!key.startsWith('app:')) return null;
  const name = key.slice(key.lastIndexOf('/') + 1).replace(/\.app$/, '');
  const [a, b, glyph, fg] = ICONS[name] ?? ['#8a8fa0', '#5a5f70', name[0]];
  const fs = glyph.length > 1 ? 34 : 46;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs><rect x="6" y="6" width="88" height="88" rx="21" fill="url(#g)"/><rect x="6.5" y="6.5" width="87" height="87" rx="20.5" fill="none" stroke="rgba(255,255,255,.18)"/><text x="50" y="52" dominant-baseline="central" text-anchor="middle" font-family="-apple-system,Helvetica,Arial" font-weight="700" font-size="${fs}" fill="${fg ?? '#fff'}">${glyph}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

// ---------- invoke ----------
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
let booted = false;

export async function mockInvoke<T>(cmd: string, args: Record<string, any> = {}): Promise<T> {
  await wait(cmd === 'clean' || cmd === 'uninstall' ? 120 : 15);
  const r = (v: unknown) => structuredClone(v) as T;
  switch (cmd) {
    case 'boot': {
      if (!booted) { booted = true; startScan(); }
      const info: BootInfo = { version: '1.0.0-mock', home: HOME, full_disk_access: fda, volume: volume(), snapshot_age_s: null };
      return r(info);
    }
    case 'rescan': startScan(); return r(null);
    case 'tree': {
      const n = byPath.get(args.path ?? '/') ?? root;
      return r(view(n, args.depth ?? 2, elapsed()));
    }
    case 'search': {
      const q = String(args.query ?? '').toLowerCase();
      const out: TreeNode[] = [];
      for (const n of byPath.values()) if (q && n.name.toLowerCase().includes(q)) out.push(view(n, 0, elapsed()));
      return r(out.sort((a, b) => b.bytes - a.bytes).slice(0, args.limit ?? 50));
    }
    case 'clutter': return r([...emitted.values()]);
    case 'clean': return r(clean(args.ids ?? [], args.paths ?? []));
    case 'undo': return r(undo(args.trashed ?? []));
    case 'apps': return r(apps());
    case 'uninstall': {
      const paths: string[] = args.paths ?? [];
      const all = paths.flatMap((p) => [p, ...(APP_DATA[p.slice(p.lastIndexOf('/') + 1).replace(/\.app$/, '')] ?? []).filter((d) => byPath.has(d))]);
      return r(clean([], all));
    }
    case 'sessions': return r(sessions());
    case 'memory': return r(memory);
    case 'memory_update': return r(memoryUpdate(args.op));
    case 'routine_run': {
      const rt = memory.routines.find((x) => x.id === args.id);
      const ids = [...emitted.values()].filter((f) => f.safety === 'safe' && rt?.select.categories?.includes(f.category)).map((f) => f.id);
      const res = clean(ids, []);
      if (rt) memoryUpdate({ type: 'set-routine', routine: { ...rt, last_run: Math.floor(Date.now() / 1000), last_freed: res.freed } });
      return r(res);
    }
    case 'reveal': case 'open': case 'quicklook': console.info(`[mock] ${cmd}`, args.path); return r(null);
    case 'open_full_disk_access': setTimeout(() => { fda = true; emit('volume', volume()); }, 1500); return r(null);
    case 'icon': return r(mockIcon(String(args.key)));
    case 'mcp_status': return r(mcp);
    case 'mcp_install': mcp = mcp.map((c) => (c.id === args.client ? { ...c, connected: true } : c)); return r(mcp);
    default: throw new Error(`mock: unknown command ${cmd}`);
  }
}

export async function mockListen(event: string, cb: Cb): Promise<() => void> {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(cb);
  return () => listeners.get(event)?.delete(cb);
}

// Handy in the console: __disko.rescan(), __disko.fda(true)
(window as any).__disko = { rescan: startScan, fda: (v: boolean) => { fda = v; emit('volume', volume()); }, leafCount: () => leafCount };
