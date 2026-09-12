// CODEPET_QA_NODE_MODULES points to an available Playwright installation.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright'));
const output = resolve('artifacts/task-lineage-qa/pet-groups');
await mkdir(output, { recursive: true });
const outDir = resolve(output, 'site');
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-groups.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1431, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 320, height: 640 } });
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://127.0.0.1:1431/frontend/qa/pet-groups.html');
  const active = page.locator('.group-active');
  const attention = page.locator('.group-attention');
  assert.equal(await attention.locator('.markdown-message').first().innerText(), '正在思考中');
  assert.equal(await page.locator('.activity-group-toggle[aria-expanded="false"]').count(), 3);
  assert.equal(await active.locator('.activity-card-slot[inert]').count(), 2);
  const bubble = active.locator('.status-pill').first();
  assert.equal(await bubble.evaluate(el => el.style.getPropertyValue('--pet-running-bubble-bg')), '#253545');
  assert.match(await bubble.evaluate(el => getComputedStyle(el).animationName), /pet-pill-breathe/);
  assert.match(await bubble.evaluate(el => getComputedStyle(el).animationName), /pet-pill-border-marquee/);
  const initialHeight = await bubble.evaluate(el => el.getBoundingClientRect().height);
  await page.screenshot({ path: resolve(output, 'collapsed.png') });
  await bubble.locator('.status-title').click();
  assert.equal(await active.locator('.activity-group-toggle').getAttribute('aria-expanded'), 'true');
  assert.equal(await attention.locator('.activity-group-toggle').getAttribute('aria-expanded'), 'false');
  assert.equal(await active.locator('[inert]').count(), 0);
  assert.equal(await bubble.evaluate(el => el.getBoundingClientRect().height), initialHeight);
  for (const message of await page.locator('.markdown-message').all()) {
    assert.ok(await message.evaluate(el => el.clientHeight <= parseFloat(getComputedStyle(el).lineHeight) + 1));
  }
  await page.screenshot({ path: resolve(output, 'expanded.png') });
  await active.locator('.activity-group-toggle').focus();
  await page.keyboard.press('Enter');
  assert.equal(await active.locator('.activity-group-toggle').getAttribute('aria-expanded'), 'false');
  const hiddenFocus = await page.evaluate(() => [...document.querySelectorAll('[inert] button')].some(el => { el.focus(); return document.activeElement === el; }));
  assert.equal(hiddenFocus, false);
  await active.locator('.dismiss-button').first().click();
  assert.equal(await active.locator('.status-pill').count(), 2);
  assert.equal(await active.locator('.activity-group-toggle').getAttribute('aria-expanded'), 'false');
  await page.getByText('切换动画', { exact: true }).click();
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.group-active .status-pill')).backgroundColor === 'rgb(37, 53, 69)');
  assert.equal(await bubble.evaluate(el => getComputedStyle(el).animationName), 'none');
  assert.equal(await bubble.evaluate(el => getComputedStyle(el).backgroundColor), 'rgb(37, 53, 69)');
  assert.equal(await bubble.evaluate(el => getComputedStyle(el).borderTopColor), 'rgb(255, 136, 51)');
  for (const width of [280, 320, 360]) {
    await page.setViewportSize({ width, height: 640 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const close = active.locator('.dismiss-button').first();
    await close.focus();
    assert.ok(await close.evaluate(el => el === document.activeElement));
  }
  assert.deepEqual(errors, []);
  console.log('Pet groups UI QA passed: independent expansion, compact content, styles, animation, dismissal, focus, and 280/320/360px layouts.');
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
