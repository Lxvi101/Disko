// Screenshots of the real Disko UI running on a fake backend.
// Run from anywhere: node video/capture/shoot.mjs [shot ...]
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import { chromium } from 'playwright';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../..');
const out = path.resolve(here, '../public/shots');
fs.mkdirSync(out, { recursive: true });
const { createServer } = await import(pathToFileURL(createRequire(path.join(repo, 'package.json')).resolve('vite')).href);
const mock = fs.readFileSync(path.join(here, 'mock-tauri.js'), 'utf8');

const server = await createServer({ root: repo, configFile: path.join(repo, 'vite.config.ts'), server: { port: 0, strictPort: false }, logLevel: 'error' });
await server.listen();
const url = server.resolvedUrls.local[0];
const browser = await chromium.launch({ headless: true });
const settle = (page, ms = 900) => page.waitForTimeout(ms);

async function open(cfg) {
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 2, colorScheme: 'dark' });
  await context.addInitScript(`window.__DISKO_MOCK__ = ${JSON.stringify(cfg)}; try { localStorage.setItem('disko.theme', 'dark'); localStorage.setItem('disko.advanced', '1'); } catch {}`);
  await context.addInitScript(mock);
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('pageerror', e.message));
  page.on('console', (m) => m.type() === 'error' && console.error('console', m.text()));
  await page.goto(url);
  await page.waitForSelector('header.app-toolbar');
  await settle(page, 1500);
  return page;
}
const store = (page, fn, arg) => page.evaluate(async ([src, a]) => {
  const { useStore } = await import('/src/store.ts');
  return new Function('useStore', 'a', `return (${src})(useStore, a)`)(useStore, a);
}, [fn.toString(), arg]);

const shots = {
  async map() {
    const page = await open({ page: 'explore' });
    await page.waitForSelector('canvas');
    await settle(page, 1500);
    await page.screenshot({ path: `${out}/map-clean.png` });
    const box = await page.locator('canvas').first().boundingBox();
    const cx = box.x + box.width / 2, cy = box.y + box.height / 2, r = Math.min(box.width, box.height) / 2;
    const a = Number(process.env.HOVER_ANGLE ?? 35) * Math.PI / 180;
    await page.mouse.move(cx + Math.cos(a) * r * 0.3, cy + Math.sin(a) * r * 0.3, { steps: 8 });
    await settle(page, 900);
    await page.screenshot({ path: `${out}/map.png` });
  },
  async suggestions() {
    const page = await open({ page: 'suggestions' });
    await page.screenshot({ path: `${out}/suggestions.png` });
  },
  async inactive() {
    const page = await open({ page: 'inactive' });
    await page.screenshot({ path: `${out}/inactive.png` });
  },
  async apps() {
    const page = await open({ page: 'apps' });
    await settle(page, 800);
    await page.screenshot({ path: `${out}/apps.png` });
  },
  async xcode() {
    const page = await open({ page: 'apps' });
    await page.locator('[data-app="xcode"]').click();
    await settle(page, 1200);
    await page.evaluate(() => { const d = document.querySelector('.app-info-strip'); const s = document.querySelector('.app-store-page'); s.scrollTop += d.getBoundingClientRect().bottom - 70; });
    // Pre-select a few duplicate simulators so the action bar shows.
    const boxes = page.locator('.xcode-row input[type="checkbox"]');
    for (let i = 0; i < 4; i++) await boxes.nth(i).check();
    await settle(page, 700);
    await page.screenshot({ path: `${out}/xcode.png` });
  },
  async quarantine() {
    const page = await open({ page: 'quarantine' });
    await page.screenshot({ path: `${out}/quarantine.png` });
  },
  async assistant() {
    const page = await open({ page: 'explore' });
    await store(page, (useStore) => {
      const H = '/Users/alex', G = 1024 ** 3;
      const actions = { suggestions: [
        { path: `${H}/Library/Developer/Xcode/DerivedData`, bytes: 18 * G, reason: 'Build output for 5 projects, rebuilt on next build.', confidence: 'high', action: 'quarantine' },
        { path: `${H}/Library/Developer/CoreSimulator/Devices`, bytes: 31 * G, reason: '9 duplicate iPhone simulators across iOS 17.5 and 18.1.', confidence: 'medium', action: 'quarantine' },
        { path: `${H}/Library/Containers/com.docker.docker`, bytes: 35 * G, reason: 'Unused images and build cache.', confidence: 'high', action: 'command', command: 'docker system prune -a' },
        { path: `${H}/.npm/_cacache`, bytes: 6.9 * G, reason: 'npm verifies and prunes its own cache.', confidence: 'high', action: 'command', command: 'npm cache verify' },
      ] };
      useStore.setState({ assistantOpen: true, messages: [
        { id: 'u1', role: 'user', text: 'where did all my space go?', createdAt: Date.now() - 60000 },
        { id: 'a1', role: 'assistant', model: 'gpt-5.5', createdAt: Date.now() - 30000, streaming: false,
          activity: [
            { id: 'c1', type: 'command', title: 'sqlite3 home.sqlite "select path,total from entries order by total desc limit 40"', status: 'done' },
            { id: 'c2', type: 'command', title: 'xcrun simctl list devices --json', status: 'done' },
            { id: 'c3', type: 'command', title: 'docker system df', status: 'done' },
          ],
          text: 'Mostly **developer stuff**. About 96 GB sits in Xcode alone:\n\n- **Simulators: 58 GB.** You have 3 copies of iPhone 16 Pro on iOS 18.1 and 3 of iPhone 15 Pro on 17.5.\n- **DerivedData: 18 GB**, all rebuildable.\n- **Docker: 35 GB**, mostly dangling images.\n\nHere is what I would clear first. Nothing moves until you confirm.',
          suggestions: actions.suggestions },
      ] });
    });
    await settle(page, 1400);
    await page.screenshot({ path: `${out}/assistant.png` });
  },
  async scan() {
    const page = await open({ page: 'explore' });
    await store(page, (useStore) => useStore.setState({ startScreen: true }));
    await settle(page, 1200);
    await page.screenshot({ path: `${out}/scan.png` });
  },
};

const wanted = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(shots);
try {
  for (const name of wanted) { await shots[name](); console.log('shot', name); }
} finally {
  await browser.close();
  await server.close();
}
