// Real layout + simulated native geometry. OS window-manager behavior still needs native QA.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/pet-module-qa');
const outDir = resolve(output, 'frame-site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1434, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER ?? 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 320, height: 584 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.exposeFunction('__resizeNativeWindow', size => page.setViewportSize(size));
  await page.goto('http://127.0.0.1:1434/frontend/qa/pet-module.html?frame');
  const settled = async () => {
    await page.waitForFunction(() => {
      const n = window.__petQa.native;
      return n.resizeCount > 0 && Math.abs(innerHeight - document.querySelector('.pet-content').getBoundingClientRect().height) <= 1;
    });
    await page.waitForTimeout(300);
  };
  const geometry = () => page.evaluate(() => {
    const n = window.__petQa.native;
    const content = document.querySelector('.pet-content').getBoundingClientRect();
    const pet = document.querySelector('.pet-avatar').getBoundingClientRect();
    return { anchorX: n.position.x + (pet.x + pet.width / 2) * n.scale, anchorY: n.position.y + (pet.y + pet.height / 2) * n.scale, height: innerHeight, top: content.top, bottom: n.position.y + n.size.height, right: n.position.x + n.size.width, y: n.position.y, count: n.resizeCount };
  });
  await settled();
  const idle = await geometry();
  assert.ok(idle.height < 180, 'idle window reserves invisible task space');
  assert.ok(Math.abs(idle.top) <= 1);
  await page.evaluate(() => { window.__petQa.tasks = [{ id: 'one', title: '正在处理任务', status: 'running', providerId: 'codex', updatedAt: Date.now() }]; });
  await page.waitForSelector('.activity-group'); await settled();
  const single = await geometry();
  assert.ok(single.height > idle.height && single.height < 310, 'single task retains the 584px preset');
  assert.ok(Math.abs(single.top) <= 1, 'transparent space above content');
  assert.equal(single.anchorY, idle.anchorY, 'task arrival moves the pet');
  assert.equal(single.right, idle.right);
  await page.screenshot({ path: resolve(output, 'window-single-task.png') });
  await page.getByRole('button', { name: '收起任务列表', exact: true }).click(); await settled();
  assert.ok((await geometry()).height <= 170);
  assert.equal((await geometry()).anchorY, idle.anchorY);
  await page.getByRole('button', { name: '展开任务列表', exact: true }).click(); await settled();
  assert.equal((await geometry()).height, single.height);
  // Content changes beyond explicit fold buttons must also resize the native frame.
  await page.evaluate(() => {
    window.__petQa.settings.pet.scale = 4;
    window.__petQa.updateSettings();
  });
  await page.waitForFunction(() => document.querySelector('.pet-avatar').getBoundingClientRect().width > 100);
  await settled();
  const largerPet = await geometry();
  assert.ok(largerPet.height > single.height, 'pet scale change does not resize the frame');
  assert.equal(largerPet.anchorY, single.anchorY);
  await page.evaluate(() => {
    window.__petQa.settings.pet.scale = 3;
    window.__petQa.updateSettings();
  });
  await page.waitForFunction(() => document.querySelector('.pet-avatar').getBoundingClientRect().width < 100);
  await settled();
  assert.equal((await geometry()).height, single.height);
  await page.getByRole('button', { name: '隐藏 正在处理任务', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('.activity-stack')); await settled();
  assert.equal((await geometry()).height, idle.height, 'dismissing the last task leaves reserved space');
  assert.equal((await geometry()).anchorY, idle.anchorY);
  await page.evaluate(() => { window.__petQa.tasks = [{ id: 'next', title: '新任务', status: 'running', providerId: 'codex', updatedAt: Date.now() }]; });
  await page.waitForSelector('.activity-group'); await settled();
  assert.equal((await geometry()).height, single.height);
  // Drag above the screen: the real content, not the previous oversized frame, reaches the top.
  await page.evaluate(() => window.__petQa.move(100, -200));
  await page.waitForFunction(() => window.__petQa.native.position.y === 40);
  assert.ok((await geometry()).height < 310);
  const topBubble = await page.locator('.activity-stack').boundingBox();
  assert.ok(topBubble.y <= 12, 'task bubble is held far below the screen edge');
  // Grow near the top. Content must stay visible, then become scrollable at its cap.
  await page.evaluate(() => { window.__petQa.tasks = Array.from({ length: 12 }, (_, i) => ({ id: `task-${i}`, title: `任务 ${i}`, status: 'running', providerId: 'codex', updatedAt: Date.now() })); });
  await page.waitForFunction(() => document.querySelector('.activity-group-count')?.textContent === '12');
  await page.locator('.activity-group-toggle').click(); await settled();
  const expanded = await geometry();
  assert.ok(expanded.height > single.height);
  assert.equal(expanded.y, 40);
  assert.ok(await page.locator('.activity-stack').evaluate(el => el.scrollHeight > el.clientHeight));
  // The separate top view starts as a one-line bar and expands on hover.
  const taskCount = await page.locator('.status-pill').count();
  await page.getByRole('button', { name: '吸附到刘海', exact: true }).click();
  await page.waitForSelector('.notch-panel');
  await page.mouse.move(-20, -20); await page.waitForTimeout(350); await settled();
  assert.equal((await geometry()).height, 38);
  assert.equal(await page.locator('.pet-stage').count(), 0);
  await page.locator('.notch-header').hover(); await page.waitForSelector('.notch-task'); await settled();
  assert.equal(await page.locator('.notch-task').count(), taskCount);
  assert.ok(await page.locator('.notch-task-list').evaluate(el => el.scrollHeight > el.clientHeight));
  await page.getByRole('button', { name: '切换为悬浮', exact: true }).focus();
  await page.screenshot({ path: resolve(output, 'window-docked-tasks.png') });
  await page.keyboard.press('Enter'); await settled();
  assert.equal(await page.locator('.status-pill').count(), taskCount);
  // Smaller work area on a negative-coordinate display, with a different DPI.
  await page.evaluate(() => {
    const q = window.__petQa;
    q.native.scale = 1.25;
    q.native.size = { width: Math.round(innerWidth * 1.25), height: Math.round(innerHeight * 1.25) };
    q.native.monitors = [{ name: 'Left', scaleFactor: 1.25, position: { x: -1600, y: -400 }, size: { width: 1600, height: 500 }, workArea: { position: { x: -1600, y: -400 }, size: { width: 1600, height: 500 } } }];
    q.move(-1400, -500);
  });
  await page.waitForFunction(() => window.__petQa.native.size.height <= 500 && window.__petQa.native.position.y >= -400 && window.__petQa.native.position.y + window.__petQa.native.size.height <= 101);
  await settled();
  assert.ok((await geometry()).height <= 400);
  assert.ok(await page.locator('.activity-stack').evaluate(el => el.scrollHeight > el.clientHeight));
  const before = await geometry(); await page.waitForTimeout(800);
  assert.equal((await geometry()).count, before.count, 'resize/move events cause a feedback loop');
  await page.getByRole('button', { name: '收起任务列表', exact: true }).focus();
  await page.keyboard.press('Enter'); await settled();
  assert.ok((await geometry()).height <= 170);
  assert.ok(Math.abs((await geometry()).anchorY - before.anchorY) <= 1, "fractional DPI moves the pet by more than one physical pixel");
  assert.deepEqual(errors, []);
  console.log(`Pet frame QA passed: idle ${idle.height}px, single task ${single.height}px; content fitting, pet scaling, last-task dismissal, character anchor, collapse/expand, top edge, capped scrolling, negative monitor/DPI, focus and stable resize events.`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
