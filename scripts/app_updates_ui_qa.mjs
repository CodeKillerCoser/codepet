// Production App + native IPC fixtures: update delivery is a capability, not a UI platform check.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/app-updates-qa'), outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/main-window.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1439, strictPort: true } });
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const page = await browser.newPage({ viewport: { width: 980, height: 700 } });
const errors = []; page.on('pageerror', error => errors.push(error.message));
async function scenario(name) { await page.goto(`http://127.0.0.1:1439/frontend/qa/main-window.html?updateCase=${name}`); }
async function about() {
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: '关于', exact: true }).click();
}
try {
  await scenario('manual');
  await page.getByRole('dialog').waitFor();
  assert.equal(await page.getByRole('button', { name: '立即升级', exact: true }).count(), 0);
  assert.equal(await page.evaluate(() => window.__updateInstallCount || 0), 0);
  await page.screenshot({ path: resolve(output, 'manual-update.png') });
  await page.getByRole('button', { name: '下载 DMG', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'detached' });
  await about();
  await page.getByText('已在浏览器打开 DMG 下载，下载后请手动安装。', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__updateInstallCount), 1);
  assert.equal(await page.getByRole('button', { name: '检查更新', exact: true }).isEnabled(), true);
  // Ignoring automatic notifications must not suppress an explicit manual check.
  await page.getByRole('button', { name: '检查更新', exact: true }).click();
  await page.getByRole('button', { name: '取消', exact: true }).click();
  await page.getByRole('button', { name: '检查更新', exact: true }).click();
  await page.getByRole('button', { name: '下载 DMG', exact: true }).waitFor();

  await scenario('installError');
  await page.getByRole('button', { name: '下载 DMG', exact: true }).click();
  await page.getByRole('dialog').getByText('Error: 无法打开安装包下载地址（模拟）', { exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '下载 DMG', exact: true }).isEnabled(), true);
  await page.getByRole('button', { name: '下载 DMG', exact: true }).click();
  assert.equal(await page.evaluate(() => window.__updateInstallCount), 2);

  await scenario('automatic');
  await page.getByRole('button', { name: '立即升级', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '下载 DMG', exact: true }).count(), 0);
  await page.getByRole('button', { name: '立即升级', exact: true }).click();
  await page.getByRole('button', { name: '正在安装…', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__updateInstallCount), 1);

  await scenario('none'); await about();
  await page.getByRole('button', { name: '检查更新', exact: true }).click();
  await page.getByText('当前已是最新版本。', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(output, 'up-to-date.png') });
  await scenario('error'); await about();
  await page.getByRole('button', { name: '检查更新', exact: true }).click();
  await page.getByRole('alert').getByText('Error: 无法获取最新发行版：网络连接失败（模拟）', { exact: true }).waitFor();
  assert.equal(await page.getByText('当前已是最新版本。', { exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  console.log('Update UI QA passed: manual DMG delivery, retry, ignored/manual checks, automatic installer, no update and network failure.');
} finally { await browser.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
