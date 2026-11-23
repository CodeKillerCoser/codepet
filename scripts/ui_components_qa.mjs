// Production UI with isolated Tauri fixtures; no installed-app data is modified.
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
import { chromium } from 'playwright';

const output = resolve('artifacts/ui-refactor'), outDir = resolve(output, 'site');
await mkdir(resolve(output, 'after'), { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/main-window.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1449, strictPort: true } });
let browser, page;
const errors = [], measurements = [];
try {
  browser = await chromium.launch({ headless: true, ...(process.env.CODEPET_QA_BROWSER ? { channel: process.env.CODEPET_QA_BROWSER } : {}) });
  page = await browser.newPage({ viewport: { width: 980, height: 700 } });
  page.setDefaultTimeout(10_000);
  page.on('pageerror', error => errors.push(error.message));
  const go = async (query = '') => {
    await page.goto(`http://127.0.0.1:1449/frontend/qa/main-window.html${query}`);
    await page.getByRole('button', { name: '设置', exact: true }).waitFor();
  };
  const button = name => page.getByRole('button', { name, exact: true });
  const navigateSettings = async label => { await button('设置').click(); await button(label).click(); };
  const screenshot = name => page.screenshot({ path: resolve(output, 'after', `${name}.png`) });
  async function fits(name) {
    const result = await page.evaluate(() => ({ viewport: innerWidth, width: document.documentElement.scrollWidth,
      background: getComputedStyle(document.querySelector('.main-theme')).getPropertyValue('--color-main-accent-solid').trim() }));
    assert.ok(result.width <= result.viewport, `${name}: no document horizontal overflow`);
    measurements.push({ name, ...result });
  }
  await go();
  await button('设置').click();
  for (const label of ['通用', '外观与桌宠', '通知', '权限', '关于']) {
    await button(label).click(); await screenshot(label); await fits(label);
  }
  await button('外观与桌宠').click();
  await button('深色模式').click();
  await page.waitForFunction(() => window.__getAppSettings().appearance.theme === 'dark');
  await screenshot('appearance-dark');
  await page.setViewportSize({ width: 820, height: 600 });
  await screenshot('appearance-dark-small'); await fits('appearance-dark-small');

  // Switch changes persist their new value, and hidden settings pages keep pending saves.
  const breathing = page.getByRole('switch', { name: '背景色呼吸灯', exact: true });
  assert.equal(await breathing.getAttribute('aria-checked'), 'true');
  await breathing.click();
  await page.waitForFunction(() => window.__getAppSettings().appearance.runningBubble.backgroundBreathing === false);
  const angle = page.getByRole('slider').first();
  await angle.fill('135');
  await button('关于').click();
  await page.waitForFunction(() => window.__getAppSettings().appearance.runningBubble.backgroundColor.includes('135deg'));
  await button('外观与桌宠').click();
  assert.equal(await breathing.getAttribute('aria-checked'), 'false');
  await button('浅色模式').click();
  await button('通知').click();
  await page.getByRole('switch', { name: '失败时响铃', exact: true }).click();
  await page.waitForFunction(() => window.__getAppSettings().notifications.ringOnFailure === false);
  await button('飞书').click();
  const nameInput = page.getByRole('textbox', { name: '名称', exact: true });
  await nameInput.fill('重构回归渠道'); await nameInput.press('Tab');
  await page.waitForFunction(() => window.__getAppSettings().notifications.robot.channels[0]?.name === '重构回归渠道');
  await page.getByText('自定义消息模板', { exact: true }).click();
  const titleInput = page.getByRole('textbox', { name: '标题', exact: true });
  await titleInput.fill('任务通知 {{status}}'); await titleInput.press('Tab');
  await page.waitForFunction(() => window.__getAppSettings().notifications.robot.template.title === '任务通知 {{status}}');

  await go(); await button('连接接入').click();
  const firstTab = page.getByRole('tab', { name: '活动接入', exact: true });
  await firstTab.focus(); await page.keyboard.press('End');
  const devicesTab = page.getByRole('tab', { name: '远程设备', exact: true });
  assert.equal(await devicesTab.getAttribute('aria-selected'), 'true');
  assert.equal(await devicesTab.evaluate(el => el === document.activeElement), true);
  assert.equal(await devicesTab.getAttribute('aria-controls'), 'connection-panel');
  const add = button('添加设备'); await add.click();
  const dialog = page.getByRole('dialog', { name: '添加设备', exact: true });
  await dialog.getByAltText('设备配对二维码').waitFor();
  const close = button('关闭添加设备对话框');
  assert.equal(await close.evaluate(el => el === document.activeElement), true);
  await page.keyboard.press('Shift+Tab');
  assert.equal(await dialog.evaluate(el => el.contains(document.activeElement)), true);
  await page.keyboard.press('Tab'); assert.equal(await close.evaluate(el => el === document.activeElement), true);
  await button('复制配对 JSON').click(); await button('已复制').waitFor();
  await screenshot('pairing-small'); await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'detached' });
  await page.waitForFunction(() => window.__pairingCancellations.length === 1);
  assert.equal(await add.evaluate(el => el === document.activeElement), true);

  // Keep the upstream workbench route, and test the legacy history component in isolation.
  await button('返回主界面').click();
  await page.getByRole('region', { name: 'Agent工作台', exact: true }).waitFor();
  await page.goto('http://127.0.0.1:1449/frontend/qa/main-window.html?uiHistory');
  const history = page.getByRole('dialog', { name: '抽取历史', exact: true });
  await history.waitFor();
  assert.equal(await history.getByRole('heading').evaluate(el => getComputedStyle(el).fontSize), '15px');
  assert.equal(await page.locator('.history-overlay').evaluate(el => getComputedStyle(el).backdropFilter), 'none');
  await page.keyboard.press('Escape'); await history.waitFor({ state: 'hidden' });

  await go('?updateCase=manual');
  const update = page.getByRole('dialog', { name: '发现新版本', exact: true });
  await update.waitFor(); await screenshot('update');
  await page.keyboard.press('Escape'); await update.waitFor({ state: 'detached' });
  await page.setViewportSize({ width: 980, height: 700 });
  await go('?updateCase=manual'); await button('下载 DMG').click();
  await page.getByRole('dialog').waitFor({ state: 'detached' });
  assert.equal(await page.evaluate(() => window.__updateInstallCount), 1);
  await navigateSettings('关于');
  await page.getByText('已在浏览器打开 DMG 下载，下载后请手动安装。', { exact: true }).waitFor();
  assert.deepEqual(errors, []);
  await writeFile(resolve(output, 'results.json'), JSON.stringify({ measurements, errors, passed: true }, null, 2));
  console.log('UI QA passed: theme, sizes, settings persistence, tabs, pairing cleanup/focus, history and update dialogs.');
} catch (error) {
  await page?.screenshot({ path: resolve(output, "failure.png") });
  if (page) await writeFile(resolve(output, "failure.txt"), await page.locator("body").innerText());
  if (page) await writeFile(resolve(output, "failure-state.json"), JSON.stringify(await page.evaluate(() => ({ settings: window.__getAppSettings(), writes: window.__settingsWrites })), null, 2));
  throw error;
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
