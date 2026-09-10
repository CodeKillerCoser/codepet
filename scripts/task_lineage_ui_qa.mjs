// Run with CODEPET_QA_NODE_MODULES pointing at a Playwright installation and Vite on port 1427.
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/task-lineage-qa');
await mkdir(output, { recursive: true });
const url = 'http://127.0.0.1:1427/frontend/qa/task-lineage.html';
let server;
if (!await fetch(url).then(response => response.ok).catch(() => false)) {
  server = spawn(process.execPath, [resolve('node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '1427'], { stdio: 'ignore', windowsHide: true });
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await fetch(url).then(response => response.ok).catch(() => false)) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}
const browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER || (process.platform === 'win32' ? 'msedge' : undefined), headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto(url);
  await page.getByText('任务抽取只把用户消息和 AI 正文作为对象，工具执行不要。', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(output, 'conversation.png'), fullPage: true });
  await page.getByRole('complementary', { name: '对话与任务导航' }).getByRole('button', { name: '任务', exact: true }).click();
  await page.getByRole('complementary', { name: '对话与任务导航' }).getByRole('button', { name: /任务谱系第一版/ }).click();
  await page.locator('.node').first().waitFor();
  assert.equal(await page.locator('.node').count(), 3);
  await page.locator('.node').nth(1).click();
  await page.getByText('三项来源回归测试已通过，证据位置保持稳定。', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(output, 'graph.png'), fullPage: true });
  const selected = await page.locator('.node.selected strong').textContent();
  await page.getByRole('button', { name: '时间泳道', exact: true }).click();
  assert.equal(await page.locator('.node.selected strong').textContent(), selected);
  await page.locator('.node').first().focus();
  assert.equal(await page.locator('.node.same-thread').count(), 2);
  await page.screenshot({ path: resolve(output, 'swimlane.png'), fullPage: true });
  await page.getByRole('button', { name: '标记已完成', exact: true }).click();
  await page.getByRole('button', { name: '重新打开', exact: true }).waitFor();
  for (const width of [980, 820, 480]) {
    await page.setViewportSize({ width, height: 700 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Horizontal page overflow at ${width}`);
    await page.screenshot({ path: resolve(output, `width-${width}.png`), fullPage: true });
    const editor = page.getByRole('complementary', { name: '事实与原始消息' }).getByRole('textbox');
    await editor.fill('继续验证当前线程');
    const send = page.getByRole('button', { name: '发送消息', exact: true });
    await send.scrollIntoViewIfNeeded();
    assert.ok(await send.evaluate(button => { const rect = button.getBoundingClientRect(); return button.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)); }), `Send action clipped at ${width}`);
  }
  await page.getByRole('button', { name: '发送消息', exact: true }).click();
  await page.getByRole('button', { name: '标记已完成', exact: true }).waitFor();
  await page.getByLabel('后台更新', { exact: true }).check();
  await page.getByLabel('后台更新', { exact: true }).uncheck();
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: ['conversation', 'node evidence', 'diagram selection', 'keyboard highlight', 'manual acceptance', 'responsive widths'], output }));
} finally { await browser.close(); server?.kill(); }
