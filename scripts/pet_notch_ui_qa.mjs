// Runs the actual two views and shared model against simulated native IPC.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/pet-notch-qa'), outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1435, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER ?? 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 320, height: 584 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.exposeFunction('__resizeNativeWindow', size => page.setViewportSize(size));
  await page.goto('http://127.0.0.1:1435/frontend/qa/pet-module.html?frame');
  await page.waitForFunction(() => document.querySelector('.pet-avatar img')?.naturalWidth > 0);
  await page.evaluate(() => {
    const q = window.__petQa;
    q.assets['/imported-pet.webp'] = document.querySelector('.pet-avatar img').src;
    q.settings.pet.kind = 'codex-atlas'; q.settings.pet.imagePath = '/imported-pet.webp'; q.updateSettings();
    q.tasks = [
      { id: 'layout', title: '调整顶部面板布局', summary: '已完成任务行和顶部圆角\n正在检查头像拖出、任务列表与悬浮窗口的切换', status: 'running', providerId: 'codex', updatedAt: Date.now() },
      { id: 'review', title: '检查角色导入的兼容性', summary: '等待确认后继续运行', status: 'waiting-input', providerId: 'claude', updatedAt: Date.now() - 60000 },
      { id: 'done', title: '完成窗口尺寸回归检查', summary: '内容自适应和位置恢复检查通过', status: 'completed', providerId: 'codex', updatedAt: Date.now() - 120000 },
    ];
  });
  await page.waitForSelector('.activity-group');
  await page.getByRole('button', { name: '吸附到刘海', exact: true }).click();
  await page.waitForSelector('.notch-panel'); await page.mouse.move(-20, -20);
  await page.waitForFunction(() => innerHeight === 38);
  assert.equal(await page.locator('.pet-avatar.portrait').count(), 1);
  assert.equal(await page.locator('.pet-action-rail').count(), 0);
  assert.match(await page.locator('.latest-title').innerText(), /调整顶部/);
  assert.equal(await page.locator('.notch-task').count(), 0);
  assert.equal(await page.locator('.notch-panel').evaluate(el => getComputedStyle(el).backgroundColor), 'rgb(0, 0, 0)');
  assert.ok(await page.evaluate(() => innerWidth <= 200));
  assert.ok(await page.locator('.pet-avatar').evaluate(el => { const frame = el.querySelector('[data-pet-frame]').getBoundingClientRect(), box = el.getBoundingClientRect(); return frame.left >= box.left && frame.right <= box.right && frame.top >= box.top; }));
  await page.screenshot({ path: resolve(output, 'collapsed.png') });
  await page.locator('.notch-header').hover(); await page.waitForSelector('.notch-task');
  await page.waitForFunction(() => innerHeight > 200 && innerWidth === 440);
  await page.locator('.notch-task').first().hover();
  await page.waitForTimeout(300); assert.equal(await page.locator('.notch-task').count(), 3);
  assert.ok(await page.locator('.notch-header').evaluate(el => Math.abs(el.getBoundingClientRect().width - el.closest('.notch-panel').getBoundingClientRect().width) < 1), 'expanded activity bar must span the same container as the list');
  assert.equal(await page.locator('.provider-badge').count(), 3);
  assert.deepEqual(await page.locator('.task-state').allTextContents(), ['进行中', '等待输入', '本轮结束']);
  assert.equal(await page.locator('time[title^="最近更新："]').count(), 3);
  assert.equal(await page.locator('.task-time').first().getAttribute('datetime'), await page.evaluate(() => new Date(window.__petQa.tasks[0].updatedAt).toISOString()));
  assert.equal(await page.locator('.progress-track, .list-heading, footer').count(), 0);
  assert.equal(await page.locator('.provider-icon img').count(), 3);
  await page.screenshot({ path: resolve(output, 'expanded.png') });
  // A brief exit and reentry must cancel the delayed close.
  await page.mouse.move(-20, -20); await page.waitForTimeout(80); await page.locator('.notch-header').hover();
  await page.waitForTimeout(300); assert.equal(await page.locator('.notch-task').count(), 3);
  await page.mouse.move(-20, -20); await page.waitForFunction(() => innerHeight === 38);
  // Keyboard entry opens the same panel; Escape returns focus to the bar.
  await page.keyboard.press('Tab'); await page.waitForSelector('.notch-details');
  await page.getByRole('button', { name: '打开主窗口', exact: true }).focus();
  await page.keyboard.press('Escape'); await page.waitForFunction(() => innerHeight === 38);
  assert.equal(await page.getByRole('button', { name: '展开任务面板', exact: true }).evaluate(el => el === document.activeElement), true);
  await page.keyboard.press('Enter'); await page.waitForSelector('.notch-details');
  await page.getByRole('button', { name: '隐藏 完成窗口尺寸回归检查', exact: true }).click();
  await page.getByRole('button', { name: '切换为悬浮', exact: true }).click();
  await page.waitForSelector('.pet-stage');
  assert.equal(await page.locator('.pet-avatar.portrait').count(), 0);
  assert.equal(await page.locator('.status-pill').count(), 2);
  assert.equal(await page.locator('[data-asset-error]').count(), 0);
  await page.screenshot({ path: resolve(output, 'desktop.png') });
  await page.getByRole('button', { name: '吸附到刘海', exact: true }).click(); await page.waitForSelector('.notch-panel');
  await page.mouse.move(-20, -20); await page.waitForFunction(() => innerHeight === 38);
  // Portrait drag uses the same native transaction and restores the full character.
  await page.evaluate(() => { window.__petQa.manualDrag = true; });
  const portrait = await page.locator('.pet-avatar').boundingBox();
  await page.mouse.move(portrait.x + 16, portrait.y + 16); await page.mouse.down();
  await page.mouse.move(portrait.x + 30, portrait.y + 24);
  await page.waitForSelector('.pet-stage');
  assert.equal(await page.locator('.activity-stack').count(), 0);
  await page.mouse.up(); await page.evaluate(() => window.__petQa.finishDrag(true));
  await page.waitForSelector('.notch-panel'); await page.mouse.move(-20, -20);
  await page.waitForFunction(() => innerHeight === 38);
  // Narrow display, long text and many tasks still fit and scroll within the panel.
  await page.evaluate(() => {
    const q = window.__petQa;
    q.tasks = Array.from({ length: 14 }, (_, i) => ({ id: `n-${i}`, title: '非常长的任务名称，用来验证单行省略与窄屏布局'.repeat(3), status: 'running', providerId: 'codex', updatedAt: Date.now() + i }));
    q.surface.availableWidth = 280; q.surface.availableHeight = 340; q.finishDrag(true);
  });
  await page.waitForFunction(() => innerWidth <= 280);
  await page.locator('.notch-header').hover(); await page.waitForFunction(() => document.querySelectorAll('.notch-task').length === 14);
  await page.waitForTimeout(200);
  assert.ok(await page.locator('.notch-task-list').evaluate(el => el.scrollHeight > el.clientHeight));
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && innerHeight <= 340));
  assert.ok(await page.locator('.task-progress').first().innerText() === '尚未收到回复内容');
  const headerTop = await page.locator('.notch-header').evaluate(el => el.getBoundingClientRect().top);
  await page.locator('.notch-task-list').evaluate(el => { el.scrollTop = el.scrollHeight; });
  assert.equal(await page.locator('.notch-header').evaluate(el => el.getBoundingClientRect().top), headerTop, 'activity bar must remain fixed while tasks scroll');
  await page.getByRole('button', { name: '切换为悬浮', exact: true }).focus();
  await page.screenshot({ path: resolve(output, 'narrow.png') });
  // Native display changes must resize both views without using the old webview size.
  const matrix = [];
  for (const [width, height] of [[1024, 640], [1440, 860], [2560, 1440], [3840, 2160]]) {
    await page.mouse.move(-20, -20);
    await page.keyboard.press('Escape');
    await page.evaluate(({ width, height }) => {
      const q = window.__petQa;
      q.native.scale = 1;
      q.native.monitors = [{ name: 'Responsive', scaleFactor: 1, position: { x: 0, y: 0 }, size: { width, height }, workArea: { position: { x: 0, y: 0 }, size: { width, height } } }];
      q.surface.availableWidth = width; q.surface.availableHeight = height;
      q.finishDrag(true);
    }, { width, height });
    await page.waitForFunction(h => innerHeight === Math.round(Math.min(44, Math.max(36, 38 * Math.sqrt(h / 860)))), height);
    assert.ok(await page.evaluate(w => innerWidth <= Math.round(Math.min(260, Math.max(160, 200 * Math.sqrt(w / 1440)))), width));
    await page.locator('.notch-header').hover();
    await page.waitForFunction(w => innerWidth === Math.round(Math.min(640, Math.max(340, 440 * Math.sqrt(w / 1440)))), width);
    await page.keyboard.press('Tab');
    assert.ok(await page.evaluate(h => innerHeight <= Math.min(h, 440) + 1, height));
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const expandedSize = await page.evaluate(() => [innerWidth, innerHeight]);
    await page.screenshot({ path: resolve(output, `screen-${width}-expanded.png`) });
    await page.getByRole('button', { name: '切换为悬浮', exact: true }).click();
    await page.waitForSelector('.pet-stage');
    await page.waitForFunction(w => innerWidth === Math.round(Math.min(360, Math.max(240, 280 * Math.sqrt(w / 1440)))), width);
    await page.waitForTimeout(200);
    assert.ok(await page.evaluate(h => innerHeight <= Math.min(h, 440) + 1, height));
    assert.ok(await page.locator('.pet-action-rail').evaluate(el => el.getBoundingClientRect().bottom <= innerHeight));
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const desktopSize = await page.evaluate(() => [innerWidth, innerHeight]);
    await page.screenshot({ path: resolve(output, `screen-${width}-desktop.png`) });
    const count = await page.evaluate(() => window.__petQa.native.resizeCount);
    await page.waitForTimeout(300);
    assert.equal(await page.evaluate(() => window.__petQa.native.resizeCount), count, 'screen sizing must not feed back from viewport');
    matrix.push({ screen: [width, height], expanded: expandedSize, desktop: desktopSize });
  }
  console.log('Responsive display matrix:', JSON.stringify(matrix));
  assert.deepEqual(errors, []);
  console.log('Notch UI QA passed: compact/hover, leave delay, keyboard/Escape, shared dismissals, imported atlas, detach/redock, screen-relative widths/heights, capped scrolling and visible actions.');
} finally { await browser?.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
