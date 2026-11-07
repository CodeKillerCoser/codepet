// Uses an existing Playwright install; browser IPC is mocked, native window behavior is not claimed.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/pet-module-qa');
const outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1433, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER ?? 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 640, height: 850 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  const url = 'http://127.0.0.1:1433/frontend/qa/pet-module.html';
  await page.goto(`${url}?library`);
  await page.waitForFunction(() => [...document.images].every(img => img.complete && img.naturalWidth));
  await page.screenshot({ path: resolve(output, 'library.png'), fullPage: true });
  await page.getByRole('button', { name: '熊猫 内置角色', exact: false }).click();
  await page.waitForFunction(() => document.querySelector('.character-preview img').src.includes('panda-'));
  for (const [name, row] of [['工作', '7'], ['等待', '6'], ['完成', '4'], ['出错', '5'], ['待机', '0']]) {
    await page.getByRole('button', { name, exact: true }).click();
    assert.equal(await page.locator('.character-preview [data-pet-frame]').getAttribute('data-row'), row);
  }
  const actor = page.locator('.character-preview button.pet-avatar');
  await actor.focus(); await page.keyboard.press('Enter');
  assert.equal(await actor.getAttribute('data-action'), 'celebrate');
  await page.getByRole('button', { name: '导入动画包', exact: true }).click();
  assert.equal(await page.locator('output').innerText(), 'package');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.waitForTimeout(900);
  const column = await page.locator('.character-preview [data-pet-frame]').getAttribute('data-column');
  await page.waitForTimeout(350);
  assert.equal(await page.locator('.character-preview [data-pet-frame]').getAttribute('data-column'), column);
  for (const width of [280, 320, 640]) {
    await page.setViewportSize({ width, height: 850 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.getByRole('button', { name: '导入图片', exact: true }).focus();
    assert.ok(await page.getByRole('button', { name: '导入图片', exact: true }).evaluate(el => document.activeElement === el));
  }
  await page.setViewportSize({ width: 320, height: 600 });
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector('.pet-frame img')?.naturalWidth > 0);
  await page.evaluate(() => {
    window.__petQa.tasks = ['running', 'waiting-input', 'completed'].map((status, i) => ({ id: String(i), title: `测试任务 ${i}`, status, providerId: 'codex', updatedAt: Date.now() }));
  });
  await page.waitForFunction(() => document.querySelector('.pet-avatar')?.dataset.action === 'waiting');
  assert.equal(await page.locator('.activity-group-toggle').count(), 3);
  await page.screenshot({ path: resolve(output, 'desktop-waiting.png') });
  // Verify real asset alpha for the current frame, including blank transparent corners.
  const hit = await page.evaluate(() => {
    const root = document.querySelector('.pet-window').getBoundingClientRect();
    const frame = document.querySelector('[data-pet-frame]').getBoundingClientRect();
    const points = [];
    for (let y = frame.top; y < frame.bottom; y += 4) for (let x = frame.left; x < frame.right; x += 4) points.push({ x: x - root.left, y: y - root.top, hit: window.__petQa.hit(x - root.left, y - root.top) });
    return { opaque: points.find(p => p.hit), transparent: points.find(p => !p.hit), root: { left: root.left, top: root.top } };
  });
  assert.ok(hit.opaque && hit.transparent);
  await page.evaluate(p => { window.__petQa.cursor = p; }, hit.transparent);
  await page.waitForFunction(() => window.__petQa.ignored);
  await page.evaluate(p => { window.__petQa.cursor = p; }, hit.opaque);
  await page.waitForFunction(() => !window.__petQa.ignored);
  await page.mouse.move(hit.opaque.x + hit.root.left, hit.opaque.y + hit.root.top);
  await page.mouse.down(); await page.mouse.move(hit.opaque.x + hit.root.left + 16, hit.opaque.y + hit.root.top + 4);
  await page.waitForFunction(() => window.__petQa.drags === 1); await page.mouse.up();
  for (const scale of [2, 3, 5]) {
    await page.evaluate(scale => { window.__petQa.settings.pet.scale = scale; window.__petQa.updateSettings(); }, scale);
    await page.waitForTimeout(100);
    for (const width of [280, 320, 360]) {
      await page.setViewportSize({ width, height: 600 });
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      const avatar = await page.locator('.pet-avatar').boundingBox();
      const rail = await page.locator('.pet-action-rail').boundingBox();
      assert.ok(avatar.x >= 0 && avatar.x + avatar.width <= width && rail.x + rail.width <= width);
      assert.ok(rail.x >= avatar.x + avatar.width, 'action rail overlaps the pet');
    }
  }
  await page.evaluate(() => { window.__petQa.settings.pet.kind = 'image'; window.__petQa.settings.pet.imagePath = '/missing.png'; window.__petQa.updateSettings(); });
  await page.waitForFunction(() => document.querySelector('[data-asset-error]'));
  assert.ok(await page.locator('.pet-frame img').evaluate(img => img.complete && img.naturalWidth > 0));
  assert.deepEqual(errors, []);
  console.log('Pet module UI QA passed: real artwork, selection, six states, keyboard petting, reduced motion, import action, alpha hit-testing, drag, fallback, and responsive layouts.');
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
