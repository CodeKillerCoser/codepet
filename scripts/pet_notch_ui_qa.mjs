// Runs the actual two views and shared model against simulated native IPC.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.CODEPET_QA_NODE_MODULES ? resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright') : 'playwright');
const output = resolve('artifacts/pet-notch-qa'), outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1435, strictPort: true } });
let browser, page;
try {
  browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER ?? 'chromium', headless: true });
  page = await browser.newPage({ viewport: { width: 320, height: 584 } });
  page.setDefaultTimeout(15000);
  const settled = phase => page.waitForFunction(expected => {
    const frame = document.querySelector('[data-dock-frame]');
    return frame?.dataset.phase === expected && !frame.hasAttribute('data-morphing');
  }, phase);
  const waitForExpandedPanel = async () => {
    await page.waitForSelector('.dock-body-clip:not([inert]) .notch-details');
    await settled('open');
  };
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
  await page.waitForFunction(() => innerHeight === 24);
  assert.equal(await page.locator('.pet-avatar.portrait').count(), 1);
  assert.equal(await page.locator('.pet-action-rail').count(), 0);
  assert.match(await page.locator('.latest-title').innerText(), /调整顶部/);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-task').count(), 0);
  assert.equal(await page.locator('.notch-backdrop').evaluate(el => getComputedStyle(el).backgroundColor), 'rgb(0, 0, 0)');
  assert.ok(await page.evaluate(() => innerWidth <= 200));
  assert.ok(await page.locator('.pet-avatar').evaluate(el => { const frame = el.querySelector('[data-pet-frame]').getBoundingClientRect(), box = el.getBoundingClientRect(); return frame.left >= box.left && frame.right <= box.right && frame.top >= box.top; }));
  await page.screenshot({ path: resolve(output, 'collapsed.png') });
  await page.locator('.notch-header').hover(); await page.waitForSelector('.notch-task');
  await page.waitForFunction(() => innerHeight > 200 && innerWidth === 440);
  await page.locator('.notch-task').first().hover();
  await page.waitForTimeout(300); assert.equal(await page.locator('.notch-task').count(), 3);
  assert.ok(await page.locator('.notch-header').evaluate(el => Math.abs(el.getBoundingClientRect().width - document.querySelector('.notch-task-list').getBoundingClientRect().width) < 1), 'header and list share the same inset container');
  const checkAlignment = () => page.evaluate(() => {
    const rect = el => el.getBoundingClientRect(), center = el => { const r = rect(el); return r.top + r.height / 2; };
    const panel = document.querySelector('.notch-panel'), css = getComputedStyle(panel);
    const avatar = document.querySelector('.dock-leading .pet-avatar');
    const headerAligned = Math.abs(center(avatar) - center(document.querySelector('.notch-summary'))) < 1;
    const rows = [...document.querySelectorAll('.notch-task:not([aria-hidden="true"])')];
    return {
      padding: [css.paddingTop, css.paddingRight, css.paddingBottom, css.paddingLeft], headerAligned,
      rowsAligned: rows.every(row => [...row.querySelectorAll('.provider-icon, .task-state, .task-time, .provider-badge, .task-attention')].every(el => Math.abs(center(el) - center(row.querySelector('h3'))) < 1)),
      sameIconSize: [...document.querySelectorAll('.provider-icon img')].every(icon => { const r = rect(icon); return Math.abs(r.width - rect(avatar).height) < 1 && Math.abs(r.height - rect(avatar).height) < 1; }),
      sameTextEdge: rows.every(row => Math.abs(rect(row.querySelector('.task-topline')).left - rect(rows[0].querySelector('.task-topline')).left) < 1),
      spacing: rows.every(row => Math.abs(rect(row.querySelector('.task-topline')).left - rect(row.querySelector('.provider-icon') || row.closest('.task-group').querySelector('.group-icon')).right - 8) < 1 && Math.abs(rect(row.querySelector('.task-progress')).top - rect(row.querySelector('.task-topline')).bottom - 8) < 1),
      noHorizontalOverflow: document.querySelector('.notch-task-list').scrollWidth <= document.querySelector('.notch-task-list').clientWidth,
      actionsInside: rows.every(row => rect(row.querySelector('.dismiss-task')).right <= rect(row.querySelector('.task-content')).right + 1),
    };
  });
  assert.deepEqual(await checkAlignment(), { padding: ['0px', '0px', '0px', '0px'], headerAligned: true, rowsAligned: true, sameIconSize: true, sameTextEdge: true, spacing: true, noHorizontalOverflow: true, actionsInside: true });
  assert.equal(await page.locator('.provider-badge').count(), 2);
  assert.deepEqual(await page.locator('.task-state').allTextContents(), ['进行中', '等待输入', '本轮结束']);
  assert.equal(await page.locator('time[title^="最近更新："]').count(), 3);
  assert.equal(await page.locator('.task-time').first().getAttribute('datetime'), await page.evaluate(() => new Date(window.__petQa.tasks[0].updatedAt).toISOString()));
  assert.equal(await page.locator('.progress-track, .list-heading, footer').count(), 0);
  assert.equal(await page.locator('.provider-icon img').count(), 3);
  assert.equal(await page.locator('.notch-panel').evaluate(el => getComputedStyle(el).borderTopLeftRadius), '0px');
  assert.equal(await page.locator('.notch-backdrop').evaluate(el => getComputedStyle(el, '::before').content), '""');
  await page.screenshot({ path: resolve(output, 'expanded.png') });
  // A brief exit and reentry must cancel the delayed close.
  await page.mouse.move(-20, -20); await page.waitForTimeout(80); await page.locator('.notch-header').hover();
  await page.waitForTimeout(300); assert.equal(await page.locator('.notch-task').count(), 3);
  await page.mouse.move(-20, -20); await page.waitForFunction(() => innerHeight === 24);
  // Start keyboard navigation before the panel. A previous mouse interaction can
  // leave Chromium's sequential focus origin after the panel even with body active;
  // pressing Tab from that implicit origin may focus browser chrome instead.
  await page.waitForFunction(() => document.querySelector('[data-dock-frame]')?.dataset.phase === 'closed');
  await page.evaluate(() => {
    const start = document.createElement('button');
    start.id = 'qa-keyboard-start';
    start.style.cssText = 'position:fixed;left:-1000px;top:0';
    document.body.prepend(start);
    start.focus();
  });
  await page.keyboard.press('Tab');
  assert.equal(await page.locator('.pet-avatar').evaluate(el => el === document.activeElement && el.matches(':focus-visible')), true, 'Tab must enter the panel from the explicit keyboard start');
  await page.locator('#qa-keyboard-start').evaluate(el => el.remove());
  await waitForExpandedPanel();
  await page.getByRole('button', { name: '打开主窗口', exact: true }).focus();
  await page.keyboard.press('Escape'); await page.waitForFunction(() => innerHeight === 24);
  assert.equal(await page.getByRole('button', { name: '展开任务面板', exact: true }).evaluate(el => el === document.activeElement), true);
  await page.keyboard.press('Enter'); await waitForExpandedPanel();
  await page.waitForFunction(() => !document.querySelector('[data-morphing]'));
  assert.equal(await page.locator('.notch-summary').evaluate(el => el === document.activeElement), true, 'native resize preparation must preserve keyboard focus');
  await page.getByRole('button', { name: '隐藏 完成窗口尺寸回归检查', exact: true }).click();
  await page.getByRole('button', { name: '切换为悬浮', exact: true }).click();
  await page.waitForSelector('.pet-stage');
  assert.equal(await page.locator('.pet-avatar.portrait').count(), 0);
  assert.equal(await page.locator('.status-pill').count(), 2);
  assert.equal(await page.locator('[data-asset-error]').count(), 0);
  await page.screenshot({ path: resolve(output, 'desktop.png') });
  await page.getByRole('button', { name: '吸附到刘海', exact: true }).click(); await page.waitForSelector('.notch-panel');
  await page.mouse.move(-20, -20); await page.waitForFunction(() => innerHeight === 24);
  // Portrait drag uses the same native transaction and restores the full character.
  await page.evaluate(() => { window.__petQa.manualDrag = true; });
  const portrait = await page.locator('.pet-avatar').boundingBox();
  await page.mouse.move(portrait.x + 16, portrait.y + 16); await page.mouse.down();
  await page.mouse.move(portrait.x + 30, portrait.y + 24);
  await page.waitForSelector('.pet-stage');
  assert.equal(await page.locator('.activity-stack').count(), 0);
  await page.mouse.up(); await page.evaluate(() => window.__petQa.finishDrag(true));
  await page.waitForSelector('.notch-panel'); await page.mouse.move(-20, -20);
  await page.waitForFunction(() => innerHeight === 24);
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
  await page.waitForFunction(() => !document.querySelector('[data-morphing]'));
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
    await page.waitForFunction(h => innerHeight === Math.round(Math.min(26, Math.max(22, 24 * Math.sqrt(h / 860)))), height);
    assert.ok(await page.evaluate(w => innerWidth <= Math.round(Math.min(260, Math.max(160, 200 * Math.sqrt(w / 1440)))), width));
    await page.locator('.notch-header').hover();
    await page.waitForFunction(w => innerWidth === Math.round(Math.min(640, Math.max(340, 440 * Math.sqrt(w / 1440)))), width);
    await page.keyboard.press('Tab');
    assert.ok(await page.evaluate(h => innerHeight <= Math.min(h, 440) + 1, height));
    await page.waitForFunction(() => !document.querySelector('main [data-morphing]'));
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
  // One UI on every screen; only the camera clearance changes.
  await page.evaluate(() => { const q = window.__petQa; q.surface = { ...q.surface, physicalNotch: false, topInset: 0 }; q.finishDrag(true); });
  await page.locator('.notch-header').hover();
  await waitForExpandedPanel();
  await page.keyboard.press('Tab');
  await page.locator('.notch-task-list').focus();
  const corners = () => page.locator('.notch-panel').evaluate(el => {
    const css = getComputedStyle(el);
    return [css.borderTopLeftRadius, css.borderTopRightRadius, css.borderBottomLeftRadius, css.borderBottomRightRadius, getComputedStyle(el.querySelector('.notch-backdrop'), '::before').content, getComputedStyle(el.querySelector('.notch-backdrop'), '::after').content];
  });
  await settled('open');
  assert.deepEqual(await corners(), ['0px', '0px', '12px', '12px', '\"\"', '\"\"']);
  await page.waitForFunction(() => innerWidth === 640 && innerHeight === 440);
  await page.screenshot({ path: resolve(output, 'top-edge-expanded.png') });
  await page.evaluate(() => { const q = window.__petQa; q.surface = { ...q.surface, physicalNotch: true, topInset: 32, notchWidth: 185 }; q.finishDrag(true); });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.notch-panel')).paddingTop === '32px');
  await settled('open');
  assert.deepEqual(await corners(), ['0px', '0px', '12px', '12px', '\"\"', '\"\"']);
  assert.ok(await page.locator('.notch-header').evaluate(el => el.getBoundingClientRect().top >= 32));
  await page.screenshot({ path: resolve(output, 'notch-fused-expanded.png') });
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  await page.waitForFunction(() => innerHeight === 58);
  await page.screenshot({ path: resolve(output, 'notch-fused-collapsed.png') });
  // Moving out of the notch removes the safe inset, not the shoulder corners.
  await page.evaluate(() => { const q = window.__petQa; q.surface = { ...q.surface, topInset: 0 }; q.finishDrag(true); });
  await page.waitForFunction(() => innerHeight === 26);
  assert.deepEqual(await corners(), ['0px', '0px', '8px', '8px', '\"\"', '\"\"']);
  await page.mouse.move(-20, -20);
  await page.locator('.notch-header').hover();
  await waitForExpandedPanel();
  await page.keyboard.press('Tab');
  await page.locator('.notch-task-list').focus();
  // Completed history stays below live/approval tasks even when it is newer.
  await page.evaluate(() => {
    const q = window.__petQa, now = Date.now();
    q.tasks = [
      ...Array.from({ length: 18 }, (_, i) => ({ id: `done-${i}`, title: `已完成任务 ${i + 1}`, summary: '已完成检查，结果已保存', status: 'completed', providerId: 'codex', updatedAt: now - i })),
      { id: 'running', title: '当前进行中的任务', summary: '正在处理最新请求', status: 'running', providerId: 'codex', updatedAt: now - 1000 },
      { id: 'approval', title: '需要授权的任务', summary: '等待你确认后继续', status: 'waiting-approval', providerId: 'claude', updatedAt: now - 2000 },
      { id: 'failed', title: '需要检查的失败任务', summary: '连接失败，请检查后重试', status: 'failed', providerId: 'codex', updatedAt: now + 1000 },
    ];
    q.surface.availableWidth = 1440; q.surface.availableHeight = 860; q.finishDrag(true);
  });
  await page.waitForFunction(() => document.querySelector('.completed-count')?.textContent === '18');
  await page.waitForFunction(() => innerWidth === 440);
  const completedToggle = page.locator('.completed-toggle');
  assert.equal(await completedToggle.getAttribute('aria-expanded'), 'false');
  assert.deepEqual(await page.locator('.notch-task:not([aria-hidden="true"]) h3').allTextContents(), ['当前进行中的任务', '需要授权的任务', '需要检查的失败任务', '已完成任务 1']);
  assert.equal(await page.locator('#notch-completed-codex .notch-task[inert]').count(), 2);
  assert.deepEqual(await checkAlignment(), { padding: ['0px', '0px', '0px', '0px'], headerAligned: true, rowsAligned: true, sameIconSize: true, sameTextEdge: true, spacing: true, noHorizontalOverflow: true, actionsInside: true });
  await page.waitForTimeout(200);
  await page.screenshot({ path: resolve(output, 'completed-stacked.png') });
  await page.evaluate(() => { window.__petQa.surface.availableWidth = 280; window.__petQa.finishDrag(true); });
  await page.waitForFunction(() => innerWidth === 280);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  assert.ok(await page.locator('#notch-completed-codex').evaluate(el => el.getBoundingClientRect().right <= innerWidth));
  await page.evaluate(() => { window.__petQa.surface.availableWidth = 1440; window.__petQa.finishDrag(true); });
  await page.waitForFunction(() => innerWidth === 440);
  await page.locator('#notch-completed-codex h3').first().click();
  assert.equal(await completedToggle.getAttribute('aria-expanded'), 'true');
  assert.equal(await page.locator('#notch-completed-codex .notch-task').count(), 18);
  assert.equal(await page.locator('#notch-completed-codex [inert]').count(), 0);
  await page.waitForFunction(() => innerHeight === 340);
  assert.ok(await page.locator('.notch-task-list').evaluate(el => el.scrollHeight > el.clientHeight));
  await page.screenshot({ path: resolve(output, 'completed-expanded.png') });
  await completedToggle.focus(); await page.keyboard.press('Space');
  assert.equal(await completedToggle.getAttribute('aria-expanded'), 'false');
  assert.equal(await completedToggle.evaluate(el => el === document.activeElement), true);
  // A completed task that resumes must leave the pile immediately.
  await page.evaluate(() => { window.__petQa.tasks[0].status = 'running'; });
  await page.waitForFunction(() => document.querySelector('.completed-count')?.textContent === '17');
  assert.equal(await page.locator('[aria-label="当前任务"] .notch-task').count(), 4);
  // Dismissing the top card must not accidentally expand the group.
  await page.getByRole('button', { name: '隐藏 已完成任务 2', exact: true }).click();
  assert.equal(await completedToggle.getAttribute('aria-expanded'), 'false');
  assert.equal(await page.locator('#notch-completed-codex h3').first().innerText(), '已完成任务 3');
  // Completed histories are isolated by agent; expanding one must not expand another.
  await page.evaluate(() => {
    const now = Date.now();
    window.__petQa.settings.pet.selectedPetId = 'builtin:panda';
    window.__petQa.settings.pet.kind = 'builtin';
    window.__petQa.updateSettings();
    window.__petQa.tasks = [
      { id: 'live', title: '调整任务面板布局', summary: '正在检查取消任务归组与单图标布局', status: 'running', providerId: 'codex', updatedAt: now },
      ...['codex', 'claude'].flatMap((providerId, group) => Array.from({ length: 3 }, (_, i) => ({ id: `${providerId}-${i}`, title: [providerId === 'codex' ? '已取消的窗口布局检查' : '完成窗口布局检查', '完成角色导入检查', '完成拖动切换检查'][i], summary: i === 0 ? '标题与消息布局检查通过\n已完成小屏和大屏回归' : '检查通过，结果已保存', status: providerId === 'codex' && i === 0 ? 'interrupted' : 'completed', providerId, updatedAt: now - 1000 * (1 + group * 3 + i) }))),
    ];
  });
  await page.waitForFunction(() => document.querySelectorAll('.completed-toggle').length === 2);
  const codexToggle = page.getByRole('button', { name: 'Codex 已完成任务（3）', exact: true });
  const claudeToggle = page.getByRole('button', { name: 'Claude Code 已完成任务（3）', exact: true });
  assert.equal(await codexToggle.getAttribute('aria-expanded'), 'false');
  assert.equal(await claudeToggle.getAttribute('aria-expanded'), 'false');
  assert.equal(await page.locator('.completed-heading').count(), 1);
  assert.equal(await page.locator('.completed-count').innerText(), '6');
  assert.equal(await page.locator('.completed .notch-task:not([inert])').count(), 2);
  assert.equal(await page.locator('#notch-active .notch-task').count(), 1);
  assert.equal(await page.locator('#notch-completed-codex .task-state').first().innerText(), '已取消');
  assert.equal(await page.locator('.completed .provider-icon').count(), 2);
  assert.equal(await page.locator('.completed .provider-badge').count(), 0);
  await page.locator('.notch-task-list').evaluate(el => el.scrollTop = 0);
  await page.keyboard.press('Tab');
  await page.locator('.notch-task-list').focus();
  await page.mouse.move(-20, -20); await page.waitForTimeout(200);
  await page.screenshot({ path: resolve(output, 'agent-stacks.png') });
  await codexToggle.click();
  assert.equal(await codexToggle.getAttribute('aria-expanded'), 'true');
  assert.equal(await claudeToggle.getAttribute('aria-expanded'), 'false');
  assert.equal(await page.locator('#notch-completed-codex .notch-task:not([inert])').count(), 3);
  assert.equal(await page.locator('#notch-completed-claude .notch-task:not([inert])').count(), 1);
  assert.equal(await page.locator('#notch-completed-codex .provider-icon').count(), 1);
  assert.equal(await page.locator('#notch-completed-codex .completed-toggle').count(), 1);
  assert.equal(await page.locator('.completed .provider-badge').count(), 0);
  assert.ok(await page.locator('#notch-completed-codex').evaluate(group => {
    const column = group.querySelector('.task-group-items').getBoundingClientRect();
    return [...group.querySelectorAll('.task-content')].every(content => {
      const box = content.getBoundingClientRect();
      return Math.abs(box.left - column.left) < 1 && Math.abs(box.right - column.right) < 1;
    });
  }), 'expanded task content must fill the whole shared right column');
  assert.deepEqual(await checkAlignment(), { padding: ['0px', '0px', '0px', '0px'], headerAligned: true, rowsAligned: true, sameIconSize: true, sameTextEdge: true, spacing: true, noHorizontalOverflow: true, actionsInside: true });
  assert.ok(await page.locator('#notch-completed-codex').evaluate(group => {
    const icon = group.querySelector('.group-icon').getBoundingClientRect();
    const heading = group.querySelector('h3').getBoundingClientRect();
    return Math.abs(icon.top + icon.height / 2 - heading.top - heading.height / 2) < 1;
  }));
  await page.screenshot({ path: resolve(output, 'agent-expanded.png') });
  // The whole expanded group keeps one icon column at narrow widths, too.
  await page.evaluate(() => { window.__petQa.surface.availableWidth = 280; window.__petQa.finishDrag(true); });
  await page.waitForFunction(() => innerWidth === 280);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  assert.equal(await page.locator('#notch-completed-codex .provider-icon').count(), 1);
  await page.screenshot({ path: resolve(output, 'agent-expanded-narrow.png') });
  await page.evaluate(() => { window.__petQa.surface.availableWidth = 1440; window.__petQa.finishDrag(true); });
  await page.waitForFunction(() => innerWidth === 440);
  // Cancelled tasks can resume and then return to the same provider group.
  await page.evaluate(() => { window.__petQa.tasks.find(t => t.id === 'codex-0').status = 'running'; });
  await page.waitForFunction(() => document.querySelector('#notch-active')?.querySelectorAll('.notch-task').length === 2);
  assert.equal(await page.locator('#notch-completed-codex .notch-task').count(), 2);
  await page.evaluate(() => { window.__petQa.tasks.find(t => t.id === 'codex-0').status = 'interrupted'; });
  await page.waitForFunction(() => document.querySelector('#notch-active')?.querySelectorAll('.notch-task').length === 1);
  assert.equal(await page.locator('#notch-completed-codex .notch-task').count(), 3);
  await codexToggle.click();
  assert.equal(await codexToggle.getAttribute('aria-expanded'), 'false');
  await codexToggle.click();
  await claudeToggle.focus(); await page.keyboard.press('Space');
  assert.equal(await claudeToggle.getAttribute('aria-expanded'), 'true');
  assert.equal(await codexToggle.getAttribute('aria-expanded'), 'true');
  // Removing an agent resets only its expansion state when it later returns.
  await page.evaluate(() => { window.__petQa.tasks = window.__petQa.tasks.filter(t => t.providerId !== 'codex'); });
  await page.waitForFunction(() => document.querySelectorAll('.completed-toggle').length === 1);
  assert.equal(await claudeToggle.getAttribute('aria-expanded'), 'true');
  await page.evaluate(() => window.__petQa.tasks.push({ id: 'return', title: '新的完成任务', summary: '已完成', status: 'completed', providerId: 'codex', updatedAt: Date.now() }));
  await page.waitForSelector('#notch-completed-codex');
  assert.equal(await page.getByRole('button', { name: 'Codex 已完成任务（1）', exact: true }).getAttribute('aria-expanded'), 'false');
  assert.equal(await page.locator('#notch-completed-codex').evaluate(el => el.classList.contains('stacked')), false);
  // Completed-only history remains available, and clearing it restores the empty state.
  await page.evaluate(() => { window.__petQa.tasks = window.__petQa.tasks.filter(t => t.status === 'completed'); });
  await page.waitForFunction(() => !document.querySelector('[aria-label="当前任务"]'));
  assert.equal(await page.locator('.empty-state').count(), 0);
  await page.evaluate(() => { window.__petQa.tasks = []; });
  await page.waitForSelector('.empty-state');
  assert.equal(await completedToggle.count(), 0);
  await page.getByRole('button', { name: '切换为悬浮', exact: true }).click();
  await page.waitForSelector('.pet-stage');
  await page.getByRole('button', { name: '吸附到刘海', exact: true }).click();
  await page.waitForSelector('.notch-panel');
  // A background WKWebView need not deliver DOM mouseenter/mouseleave.
  // Feed only the native pointer API: hover must work without focus or a click.
  await page.mouse.move(-20, -20);
  await page.evaluate(() => {
    document.activeElement?.blur();
    Object.defineProperty(document, 'hasFocus', { configurable: true, value: () => false });
    window.__petQa.cursorScreen = null; window.__petQa.cursor = { x: -20, y: -20 };
  });
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  await page.evaluate(() => { const q=window.__petQa; q.cursorScreen = { x: q.native.position.x / q.native.scale + innerWidth / 2, y: q.native.position.y / q.native.scale + 12 }; });
  await waitForExpandedPanel();
  assert.equal(await page.evaluate(() => document.hasFocus()), false);
  assert.equal(await page.evaluate(() => window.__petQa.ignored), false);
  assert.equal(await page.evaluate(() => document.activeElement === document.body), true, 'background hover must not take keyboard focus');
  await page.locator('.notch-summary').focus();
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  await page.waitForTimeout(360);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 0, 'background polling must respect Escape until the pointer leaves');
  await page.evaluate(() => { window.__petQa.cursorScreen = null; window.__petQa.cursor = { x: -20, y: -20 }; });
  await page.waitForFunction(() => window.__petQa.ignored);
  await page.evaluate(() => { const q=window.__petQa; q.cursorScreen = { x: q.native.position.x / q.native.scale + innerWidth / 2, y: q.native.position.y / q.native.scale + 12 }; });
  await waitForExpandedPanel();
  await page.evaluate(() => { document.activeElement?.blur(); window.__petQa.cursorScreen = null; window.__petQa.cursor = { x: -20, y: -20 }; });
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  await page.evaluate(() => { delete document.hasFocus; });
  // Hover dwell, press-before-open, and both horizontal drag presentations.
  await page.evaluate(() => { const q = window.__petQa; q.manualDrag = true; q.dragAlongTop = true; document.activeElement?.blur(); });
  await page.mouse.move(-20, -20);
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  const header = page.locator('.notch-panel');
  await header.dispatchEvent('mouseenter');
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 0, 'hover must not expand immediately');
  await header.dispatchEvent('mouseleave');
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 0, 'brief hover must cancel expansion');
  const smallAvatar = await page.locator('.pet-avatar').boundingBox();
  const smallX = smallAvatar.x + smallAvatar.width / 2, smallY = smallAvatar.y + smallAvatar.height / 2;
  await page.mouse.move(smallX, smallY); await page.mouse.down();
  await page.mouse.move(smallX + 12, smallY);
  await page.waitForFunction(() => window.__petQa.surface.dragging);
  await page.waitForTimeout(360);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 0, 'drag begun before dwell must remain collapsed');
  await page.mouse.up(); await page.evaluate(() => window.__petQa.finishDrag(true));
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 0, 'release must not resurrect the cancelled dwell');
  await page.mouse.move(-20, -20); await page.locator('.notch-header').hover();
  await waitForExpandedPanel();
  assert.equal(await header.evaluate(el => getComputedStyle(el).animationName), 'none');
  await page.waitForTimeout(300);
  const largeAvatar = await page.locator('.pet-avatar').boundingBox();
  const largeX = largeAvatar.x + largeAvatar.width / 2, largeY = largeAvatar.y + largeAvatar.height / 2;
  const expandedHeight = await page.evaluate(() => innerHeight);
  await page.mouse.move(largeX, largeY); await page.mouse.down(); await page.mouse.move(largeX + 12, largeY);
  await page.waitForFunction(() => window.__petQa.surface.dragging);
  await page.mouse.move(-20, -20); await page.waitForTimeout(360);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 1, 'expanded drag must ignore mouseleave and close timer');
  assert.equal(await page.evaluate(() => innerHeight), expandedHeight);
  await page.mouse.move(largeX, largeY); await page.mouse.up();
  await page.evaluate(() => window.__petQa.finishDrag(true));
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 1, 'horizontal drag release keeps expanded layout');
  // Titles, status, action buttons, and padded header space all share the drag region.
  for (const selector of ['.latest-title', '.latest-status', '.panel-actions button:first-child', '[data-pet-drag-region]']) {
    const box = await page.locator(selector).boundingBox();
    const x = selector === '[data-pet-drag-region]' ? box.x + 3 : box.x + box.width / 2;
    const y = selector === '[data-pet-drag-region]' ? box.y + 3 : box.y + box.height / 2;
    const before = await page.evaluate(() => window.__petQa.drags);
    await page.mouse.move(x, y); await page.mouse.down(); await page.mouse.move(x + 6, y);
    await page.waitForFunction(n => window.__petQa.drags > n, before);
    await page.mouse.up(); await page.evaluate(() => window.__petQa.finishDrag(true));
    assert.equal(await page.locator('.dock-body-clip:not([inert]) .notch-details').count(), 1, `dragging ${selector} must not trigger its click action`);
    assert.equal(await page.locator('.pet-stage').count(), 0);
  }
  // Keep the header nodes alive; only the backdrop and details may be animated/clipped.
  await page.evaluate(() => { window.__stableHeader = ['.pet-avatar', '.latest-title', '.latest-status'].map(s => document.querySelector(s)); });
  await page.getByRole('button', { name: '收起任务面板', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('.dock-body-clip:not([inert]) .notch-details'));
  await page.getByRole('button', { name: '展开任务面板', exact: true }).click();
  await waitForExpandedPanel();
  assert.equal(await page.evaluate(() => window.__stableHeader.every(el => el.isConnected)), true);
  assert.deepEqual(await header.evaluate(el => { const s = getComputedStyle(el); return [s.animationName, s.clipPath, s.opacity]; }), ['none', 'none', '1']);
  assert.equal(await page.locator('.notch-header').evaluate(el => getComputedStyle(el).opacity), '1');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  assert.equal(await page.locator('.notch-backdrop').evaluate(el => getComputedStyle(el).animationName), 'none');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  console.log('Responsive display matrix:', JSON.stringify(matrix));
  assert.deepEqual(errors, []);
  console.log('Notch UI QA passed: compact/hover, keyboard/Escape, shared dismissals, imported atlas, detach/redock, responsive bounds, corners, live-first order, completed stacking/toggle/resume, capped scrolling and visible actions.');
} catch (error) {
  if (page) {
    await page.screenshot({path: resolve(output, 'failure.png')}).catch(() => {});
    await writeFile(resolve(output, 'failure.json'), JSON.stringify(await page.evaluate(() => ({
      focus: { tag: document.activeElement?.tagName, className: document.activeElement?.className, label: document.activeElement?.getAttribute('aria-label') }, hasFocus: document.hasFocus(),
      frame: document.querySelector('[data-dock-frame]')?.outerHTML,
      viewport: [innerWidth, innerHeight], native: window.__petQa?.native,
    })), null, 2)).catch(diagnosticError => console.error('Could not save failure state:', diagnosticError));
  }
  throw error;
} finally { await browser?.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
