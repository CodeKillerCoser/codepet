import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';

const [url, outputArg] = process.argv.slice(2);
if (!url || !outputArg) throw new Error('Usage: node ui_qa.mjs URL OUTPUT_DIRECTORY');
const output = path.resolve(outputArg);
const { chromium } = await import(process.env.CODEPET_PLAYWRIGHT_MODULE || 'playwright');
const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.CODEPET_CHROMIUM_EXECUTABLE,
});
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 960 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url, { waitUntil: 'networkidle' });
  await page.screenshot({ path: path.join(output, 'desktop.png'), fullPage: true });
  await page.getByRole('button', { name: 'drawer', exact: true }).click();
  const querySwitchWorked = (await page.locator('.query-text').innerText()).includes('抽屉');
  await page.locator('#graph').scrollIntoViewIfNeeded();
  const position = await page.evaluate(() => {
    const node = nodes.find(item => item.type === 'Entity');
    const bounds = canvas.getBoundingClientRect();
    return { x: bounds.x + node.x * w, y: bounds.y + node.y * h, name: node.name };
  });
  await page.mouse.click(position.x, position.y);
  const nodeSelectionWorked = (await page.locator('#node-title').innerText()) === position.name;
  await page.locator('#graph').screenshot({ path: path.join(output, 'graph.png') });
  await page.getByLabel('图展示层级').selectOption('raw');
  const rawViewWorked = await page.evaluate(() => nodes.length === g.nodes.length);
  await page.getByLabel('图展示层级').selectOption('runs');
  await page.getByLabel('选择图节点').focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  const keyboardFocusWorked = await page.locator('#graph-node').evaluate(node => document.activeElement === node);
  await page.getByLabel('选择图节点').selectOption({ index: 1 });
  const expectedNode = await page.locator('#graph-node').evaluate(select => nodes.find(node => node.id === select.value).name);
  const nodePickerWorked = (await page.locator('#node-title').innerText()) === expectedNode;
  const semanticEdges = await page.evaluate(() => g.links.filter(edge => edge.edge_class === 'semantic').length);
  if (semanticEdges) {
    await page.getByLabel('图展示层级').selectOption('semantic');
    await page.getByLabel('按 Run 筛选语义关系').selectOption('');
    assert.equal(await page.evaluate(() => links.length), semanticEdges);
  }
  let processedPreviewWorked = null;
  if (await page.locator('#processed-docs button').count()) {
    await page.locator('#processed-docs button').first().click();
    processedPreviewWorked = (await page.locator('#processed-preview').innerText()).includes('原文证据');
    assert.equal(processedPreviewWorked, true);
  }
  await page.locator('#docs button').first().click();
  const docPreviewWorked = await page.locator('#doc-preview').isVisible();
  const desktopOverflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(url, { waitUntil: 'networkidle' });
  await page.screenshot({ path: path.join(output, 'mobile.png'), fullPage: true });
  const mobileOverflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
  const native = await browser.newPage({ viewport: { width: 1280, height: 960 } });
  const nativeErrors = [];
  native.on('pageerror', error => nativeErrors.push(error.message));
  await native.goto(new URL('cognee-graph.html', url).href, { waitUntil: 'networkidle' });
  await native.screenshot({ path: path.join(output, 'cognee-native.png') });
  const result = {
    errors, querySwitchWorked, nodeSelectionWorked, keyboardFocusWorked, nodePickerWorked, rawViewWorked, docPreviewWorked,
    processedPreviewWorked, semanticEdges,
    desktopOverflow, mobileOverflow, nativeErrors, nativeTitle: await native.title(),
    nativeCanvasOrSvg: await native.locator('canvas,svg').count(),
  };
  await fs.writeFile(path.join(output, 'ui-qa.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  assert.deepEqual(errors, []);
  assert.equal(querySwitchWorked, true);
  assert.equal(nodeSelectionWorked, true);
  assert.equal(keyboardFocusWorked, true);
  assert.equal(nodePickerWorked, true);
  assert.equal(rawViewWorked, true);
  assert.equal(docPreviewWorked, true);
  assert.equal(desktopOverflow, false);
  assert.equal(mobileOverflow, false);
  assert.deepEqual(nativeErrors, []);
  assert.ok(result.nativeCanvasOrSvg > 0);
} finally {
  await browser.close();
}
