// Actual pointer/keyboard events: programmatic element.click() misses WebKit blur ordering.
import { chromium, webkit } from 'playwright';
import { build, preview } from 'vite';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';

const output = resolve('artifacts/codex-workbench-qa/project-picker');
await mkdir(output, { recursive: true });
const outDir = resolve(output, 'site');
await build({ build: { outDir, emptyOutDir: true, rollupOptions: { input: resolve('frontend/qa/codex-workbench.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1439, strictPort: true } });
const reports = [];
try {
  for (const [engine, type] of Object.entries({ chromium, webkit })) {
    const browser = await type.launch({ headless: true });
    try {
      const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
      page.setDefaultTimeout(10_000);
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto('http://127.0.0.1:1439/frontend/qa/codex-workbench.html');
      const input = page.getByLabel('搜索并选择项目', { exact: true });
      const popup = page.getByRole('group', { name: '项目选项', exact: true });
      const confirm = page.getByRole('button', { name: '确认', exact: true });
      await confirm.waitFor();
      await input.click();
      const option = popup.getByRole('button', { name: 'Other project', exact: false });
      const box = await option.boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.mouse.down();
      await page.waitForTimeout(100);
      assert.ok(await popup.isVisible(), `${engine}: popup survives pointer-down until click`);
      await page.mouse.up();
      await page.getByRole('button', { name: '移除 Other project', exact: true }).waitFor();
      assert.ok(await input.evaluate(element => element === document.activeElement));
      await page.getByRole('button', { name: '移除 Other project', exact: true }).click();
      await input.fill('OTHER');
      await page.keyboard.press('ArrowDown');
      assert.ok(await option.evaluate(element => element === document.activeElement));
      await page.keyboard.press('Enter');
      await page.getByRole('button', { name: '移除 Other project', exact: true }).waitFor();
      assert.ok(await input.evaluate(element => element === document.activeElement), 'Keyboard selection returns to search');
      assert.equal(await input.inputValue(), '');
      assert.equal(await page.locator('.tags .tag').count(), 3);
      await page.keyboard.press('Escape');
      assert.ok(await popup.isHidden());
      await input.click();
      await popup.waitFor();
      await popup.getByRole('button', { name: '清空选择', exact: true }).click();
      assert.equal(await page.locator('.tags .tag').count(), 0);
      assert.ok(await confirm.isDisabled());
      assert.ok(await input.evaluate(element => element === document.activeElement));
      await popup.getByRole('button', { name: 'Code Pet', exact: false }).click();
      assert.equal(await page.locator('.tags .tag').count(), 1);
      assert.ok(await confirm.isEnabled());
      await page.getByLabel('刷新项目列表', { exact: true }).focus();
      assert.ok(await popup.isHidden(), 'Moving focus outside still dismisses the popup');
      const calls = await page.evaluate(() => window.__workbenchCalls);
      const saves = calls.filter(call => call.op === 'saveSelection');
      assert.deepEqual(saves.map(call => call.projectIds), [
        ['codepet', 'runtime', 'other'], ['codepet', 'runtime'],
        ['codepet', 'runtime', 'other'], [], ['codepet'],
      ], 'Every selection is saved exactly once and in order');
      assert.equal(calls.filter(call => call.op === 'scan').length, 0, 'Selection never starts a scan');
      await confirm.click();
      await page.locator('.worktree-row').first().waitFor();
      assert.deepEqual((await page.evaluate(() => window.__workbenchCalls)).filter(call => call.op === 'scan').map(call => call.projectIds), [['codepet']]);
      await input.click();
      await popup.getByRole('button', { name: 'Agent Runtime', exact: false }).click();
      await confirm.click();
      await page.waitForFunction(() => document.querySelectorAll('.worktree-row').length === 8);
      assert.deepEqual((await page.evaluate(() => window.__workbenchCalls)).filter(call => call.op === 'scan').map(call => call.projectIds), [['codepet'], ['codepet', 'runtime']]);
      await page.getByLabel('筛选项目', { exact: true }).click();
      await page.getByLabel('Agent Runtime', { exact: true }).check();
      assert.equal(await page.locator('.worktree-row').count(), 1, 'Result project filtering works too');
      assert.ok((await page.locator('.worktree-row .project-name').innerText()).includes('Agent Runtime'));
      await page.getByLabel('Agent Runtime', { exact: true }).uncheck();
      assert.equal(await page.locator('.worktree-row').count(), 8);
      await page.screenshot({ path: resolve(output, `after-${engine}-desktop.png`) });
      assert.deepEqual(errors, []);
      reports.push({ engine, input: 'mouse-and-keyboard', saves: saves.map(call => call.projectIds), scanProjectIds: [['codepet'], ['codepet', 'runtime']], success: true });
      await page.close();

      const touchContext = await browser.newContext({ hasTouch: true, viewport: { width: 390, height: 900 } });
      const touchPage = await touchContext.newPage();
      touchPage.setDefaultTimeout(10_000);
      await touchPage.goto('http://127.0.0.1:1439/frontend/qa/codex-workbench.html');
      await touchPage.getByLabel('搜索并选择项目').tap();
      await touchPage.getByRole('button', { name: 'Other project', exact: false }).tap();
      await touchPage.getByRole('button', { name: '移除 Other project', exact: true }).waitFor();
      await touchPage.getByRole('button', { name: '清空选择', exact: true }).tap();
      assert.equal(await touchPage.locator('.tags .tag').count(), 0);
      await touchPage.getByRole('button', { name: 'Code Pet', exact: false }).tap();
      await touchPage.getByRole('button', { name: '移除 Code Pet', exact: true }).waitFor();
      await touchPage.screenshot({ path: resolve(output, `after-${engine}-touch.png`) });
      reports.push({ engine, input: 'touch', success: true });
      await touchContext.close();
    } finally { await browser.close(); }
  }
  await writeFile(resolve(output, 'report.json'), JSON.stringify(reports, null, 2));
  console.log('Project picker passed in Chromium and WebKit: mouse, touch, keyboard, clear/remove, focus, saved selection, explicit scan and result project filters.');
} finally { await server.close(); }
