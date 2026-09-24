// Run with node tests/explore.browser.mjs (Playwright must be available via NODE_PATH or locally).
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'vite';
const { chromium } = createRequire(import.meta.url)('playwright');
const server = await createServer({
  server: { port: 0, strictPort: false },
  plugins: [{ name: 'explore-regression-fixture', configureServer(server) {
    server.middlewares.use('/__explore-test', async (_req, res) => {
      res.setHeader('Content-Type', 'text/html');
      res.end(await server.transformIndexHtml('/__explore-test', '<html><body><script type="module" src="/tests/fixtures/explore.tsx"></script></body></html>'));
    });
  } }],
});
await server.listen();
let browser;
try {
  browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH });
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(`${server.resolvedUrls.local[0]}__explore-test`);
  await page.waitForFunction(() => window.exploreTest && document.querySelector('canvas'));
  await page.waitForFunction(() => {
    const canvas = document.querySelector('canvas');
    return canvas && Math.abs(canvas.getBoundingClientRect().width - parseFloat(canvas.style.width)) < 0.01;
  });
  const check = (snapshot, name, row) => {
    assert.equal(snapshot.title, name, JSON.stringify(snapshot));
    assert.ok(snapshot.rows.some(text => text.includes(row)), JSON.stringify(snapshot));
    assert.ok(snapshot.total, 'Folder size must be present');
  };
  for (const advanced of [false, true]) {
    await page.evaluate(value => window.exploreTest.advanced(value), advanced);
    // Assert in the same commit as the rows, without waiting for a title animation.
    for (let i = 0; i < 8; i++) {
      check(await page.evaluate(() => window.exploreTest.hover('/Updates')), 'Updates', 'Installer');
      check(await page.evaluate(() => window.exploreTest.hover('/Applications')), 'Applications', 'Topaz Photo.app');
    }
    await page.locator('.legend-heading button').click();
    check(await page.evaluate(() => window.exploreTest.snapshot()), 'Macintosh HD', 'Applications');
  }
  await page.evaluate(() => window.exploreTest.advanced(false));
  await page.evaluate(() => window.exploreTest.navigate('/Updates'));
  await page.waitForFunction(() => window.exploreTest.snapshot().title === 'Updates');
  await page.evaluate(() => window.exploreTest.resolve('/Updates'));
  await page.waitForTimeout(50);
  await page.evaluate(() => window.exploreTest.navigate('/Applications'));
  await page.waitForFunction(() => window.exploreTest.snapshot().title === 'Applications');
  // The new subtree is still pending: the previous folder must not leak into the list.
  check(await page.evaluate(() => window.exploreTest.snapshot()), 'Applications', 'Topaz Photo.app');
  await page.evaluate(() => window.exploreTest.resolve('/Applications'));
  await page.waitForTimeout(50);
  check(await page.evaluate(() => window.exploreTest.snapshot()), 'Applications', 'Topaz Photo.app');
  assert.deepEqual(errors, []);
  console.log('PASS: rapid hover, heading reset, both detail modes, and delayed folder navigation');
} finally {
  await browser?.close();
  await server.close();
}
