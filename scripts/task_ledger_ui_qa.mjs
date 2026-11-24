import {chromium, webkit} from 'playwright';
import {createServer} from 'vite';
import {mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import assert from 'node:assert/strict';

const engine = process.env.CODEPET_QA_ENGINE ?? 'chromium';
assert.ok(['chromium', 'webkit'].includes(engine));
const output = resolve('artifacts/task-ledger-qa/layout', engine === 'webkit' ? 'webkit' : '.');
await mkdir(output, {recursive: true});
const server = await createServer({server: {host: '127.0.0.1', port: 1438, strictPort: true}});
await server.listen();
const browser = await ({chromium, webkit})[engine].launch({headless: true});
const errors = [];
try {
  const page = await browser.newPage({viewport: {width: 1280, height: 920}});
  page.on('pageerror', e => errors.push(e.message));
  const closeChanges=async()=>{await page.getByRole('button',{name:'关闭代码变更',exact:true}).click();await page.locator('.app-side-panel').waitFor({state:'detached'});};
  const route = scenario => `http://127.0.0.1:1438/frontend/qa/task-ledger.html${scenario ? `?scenario=${scenario}` : ''}`;
  await page.goto(route()); await page.locator('.task-row').waitFor();
  for (const name of ['目标与方案', '工作区', '验收清单', '活动时间线']) {
    assert.equal(await page.getByRole('region', {name, exact: true}).count(), 1);
  }
  assert.equal(await page.getByRole('button', {name: /^(更新执行|结束执行)$/}).count(), 0);
  assert.equal(await page.locator('.task-context .kind-tag,.criterion .fold-icon,.test-status').count(), 0);
  assert.equal(await page.locator('.run-card').count(), 2);
  const positions = await page.locator('.task-heading,.task-background,.task-context,.task-divider,.run-navigation').evaluateAll(es => es.map(e => e.getBoundingClientRect().top));
  assert.ok(positions.every((value, i) => !i || value > positions[i - 1]), 'Header order and separator');
  await page.locator('.run-card').first().click();
  assert.equal(await page.locator('.record-result').count(), 0, 'History read only');
  await page.locator('.run-card').last().focus(); await page.keyboard.press('Home');
  assert.equal(await page.locator('.run-card').first().getAttribute('aria-selected'), 'true');
  await page.keyboard.press('End');
  assert.equal(await page.locator('.run-card').last().getAttribute('aria-selected'), 'true');
  await page.getByRole('button', {name: '查看变更', exact: true}).click();
  assert.ok(await page.locator('.change-file').count() > 0);
  if(await page.locator('.commit-changes').count()){await page.locator('.commit-changes>summary').first().click();await page.getByText('committed-change.ts',{exact:true}).first().waitFor();}
  await closeChanges();

  for (const mode of ['light', 'dark']) {
    await page.evaluate(mode => window.__setTaskTheme(mode), mode);
    for (const width of [1280, 820, 600, 390]) {
      await page.setViewportSize({width, height: 920});
      const back = page.getByRole('button', {name: '← 返回任务列表', exact: true});
      if (await back.isVisible()) await back.click();
      await page.screenshot({path: resolve(output, `${mode}-${width}-list.png`), fullPage: true});
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.locator('.task-row').click();
      assert.ok(await page.locator('.task-heading h3').evaluate(e => e === document.activeElement));
      await page.screenshot({path: resolve(output, `${mode}-${width}-detail.png`), fullPage: true});
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      const criterion = page.locator('.criterion').first();
      if (!(await criterion.getAttribute('open') !== null)) await criterion.locator(':scope>summary').click();
      await criterion.getByRole('button', {name: '记录结果', exact: true}).click();
      const dialog = page.getByRole('dialog', {name: '记录验收结果', exact: true});
      await dialog.waitFor();
      assert.equal(await dialog.evaluate(e => getComputedStyle(e).colorScheme), mode);
      assert.ok(await dialog.evaluate(e => e.parentElement.classList.contains('app-window-host')));
      await page.getByLabel('证据或说明', {exact: true}).focus();
      await page.screenshot({path: resolve(output, `${mode}-${width}-result-editor.png`)});
      await page.keyboard.press('Escape'); await dialog.waitFor({state: 'detached'});
      assert.ok(await criterion.locator(':scope>summary').evaluate(e => e === document.activeElement));
      if (width <= 600) {
        await back.click();
        assert.ok(await page.locator('.task-row').evaluate(e => e === document.activeElement));
      }
    }
  }
  await page.setViewportSize({width: 1280, height: 920});
  await page.locator('.task-row').click();
  await page.getByRole('button', {name: '修改任务信息', exact: true}).click();
  await page.getByRole('dialog').waitFor();
  assert.equal(await page.getByLabel('执行类型', {exact: true}).count(), 0);
  await page.keyboard.press('Escape'); await page.getByRole('dialog').waitFor({state: 'detached'});

  // Busy state prevents cancellation and failure preserves the single-item draft.
  const criterion = page.locator('.criterion').first();
  await page.evaluate(() => { window.__holdTaskSave = true; });
  await criterion.getByRole('button', {name: '记录结果', exact: true}).click();
  await page.getByLabel('实际验收版本').fill('1.3.0'); await page.getByLabel('验收环境').fill('macOS');
  await page.getByLabel('证据或说明').fill('待保存证据');
  await page.getByLabel('验收状态', {exact:true}).selectOption('passed');
  await page.getByRole('button', {name: '保存此项结果', exact: true}).click();
  await page.waitForFunction(() => document.querySelector('.form-controls')?.disabled);
  await page.keyboard.press('Escape'); assert.ok(await page.getByRole('dialog').isVisible());
  assert.ok(await page.getByRole('button', {name: '关闭验收记录'}).isDisabled());
  await page.evaluate(() => { window.__holdTaskSave = false; window.__rejectTaskSave(); });
  await page.waitForFunction(() => !document.querySelector('.form-controls')?.disabled);
  assert.equal(await page.getByLabel('证据或说明').inputValue(), '待保存证据');
  await page.keyboard.press('Escape'); await page.getByRole('dialog').waitFor({state: 'detached'});

  await page.getByLabel('更多任务操作').click();
  await page.getByRole('button', {name: '接入 Agent', exact: true}).click();
  const prompt = page.getByRole('textbox', {name: 'Agent 使用提示词', exact: true});
  await prompt.waitFor(); assert.ok((await page.getByRole('dialog').innerText()).includes('X-CodePet-Agent'));
  const defaultText = await prompt.inputValue();
  await prompt.fill('按当前项目约定在关键节点登记。');
  await page.getByRole('button', {name: '保存提示词', exact: true}).click();
  await page.getByRole('status').filter({hasText: '提示词已保存'}).waitFor();
  await page.keyboard.press('Escape'); await page.getByRole('dialog').waitFor({state: 'detached'});
  assert.ok(await page.getByLabel('更多任务操作').evaluate(e => e === document.activeElement));
  await page.getByLabel('更多任务操作').click(); await page.getByRole('button', {name: '接入 Agent', exact: true}).click();
  assert.equal(await prompt.inputValue(), '按当前项目约定在关键节点登记。');
  await page.evaluate(() => window.__promptConflict());
  await prompt.fill('冲突时保留此草稿'); await page.getByRole('button', {name: '保存提示词', exact: true}).click();
  await page.getByRole('alert').filter({hasText: 'revision='}).waitFor();
  assert.equal(await prompt.inputValue(), '冲突时保留此草稿');
  await page.keyboard.press('Escape'); await page.getByRole('dialog').waitFor({state: 'detached'});
  await page.getByLabel('更多任务操作').click(); await page.getByRole('button', {name: '接入 Agent', exact: true}).click();
  await page.getByRole('button', {name: '恢复默认', exact: true}).click();
  await page.getByRole('status').filter({hasText: '已恢复默认'}).waitFor(); assert.equal(await prompt.inputValue(), defaultText);
  await page.screenshot({path: resolve(output, 'editable-agent-prompt.png')});
  await page.keyboard.press('Escape');

  // Regression: small horizontal overflow still renders a full-width scrollbar on macOS.
  await page.goto(route('overflow')); await page.locator('.task-row').nth(5).waitFor();
  for (const mode of ['light', 'dark']) {
    await page.evaluate(mode => window.__setTaskTheme(mode), mode);
    for (const width of [1280, 820, 390]) {
      await page.setViewportSize({width, height: 920});
      const back = page.getByRole('button', {name: '← 返回任务列表', exact: true});
      if (await back.isVisible()) await back.click();
      const titles = await page.locator('.task-row strong').evaluateAll(es => es.map(e => {
        const s = getComputedStyle(e);
        return {height:e.getBoundingClientRect().height,lineHeight:parseFloat(s.lineHeight),whiteSpace:s.whiteSpace,title:e.title,text:e.textContent};
      }));
      for (const title of titles) {
        assert.equal(title.whiteSpace, 'nowrap');
        assert.ok(title.height <= title.lineHeight + 1, 'Task title remains one line');
        assert.equal(title.title, title.text, 'Full title is retained');
      }
      await page.locator('.task-row').first().click();
      const detail = page.locator('.task-run-scroll');
      const assertFits = async () => {
        const bounds = await detail.evaluate(e => ({client:e.clientWidth,scroll:e.scrollWidth,bar:getComputedStyle(e).scrollbarWidth,webkit:getComputedStyle(e,'::-webkit-scrollbar').display}));
        assert.ok(bounds.scroll <= bounds.client + 1, `${engine}/${mode}/${width} detail overflow: ${JSON.stringify(bounds)}`);
        assert.ok(bounds.bar === 'none' || bounds.webkit === 'none', 'Detail bar hidden');
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      };
      await assertFits();
      await page.getByRole('button', {name: '查看变更', exact: true}).click();
      await assertFits();
      await closeChanges();
      if (width === 1280) {
        await detail.evaluate(e => {e.scrollTop = 0;}); await detail.focus();
        await page.keyboard.press('PageDown');
        await page.waitForFunction(() => document.querySelector('.task-run-scroll').scrollTop > 0);
      }
      await page.screenshot({path:resolve(output, `compact-${mode}-${width}.png`),fullPage:true});
    }
  }
  await page.setViewportSize({width:1280,height:920});
  await page.goto(route('overflow&manyRuns=1')); await page.locator('.run-card').nth(11).waitFor();
  assert.ok(await page.locator('.run-navigation').evaluate(e=>e.scrollWidth<=e.clientWidth+1), 'Run cards wrap instead of horizontal scrolling');
  await page.locator('.run-card').last().focus(); await page.keyboard.press('Home');
  assert.equal(await page.locator('.run-card').first().getAttribute('aria-selected'),'true');
  await page.keyboard.press('End'); assert.equal(await page.locator('.run-card').last().getAttribute('aria-selected'),'true');

  await page.goto(route('dense')); await page.locator('.task-row').nth(7).waitFor();
  assert.equal(await page.locator('.criterion').count(), 3);
  assert.equal(await page.locator('.check-mark.passed').count(), 1);
  assert.equal(await page.locator('.check-mark.failed').count(), 1);
  for (const mode of ['light', 'dark']) {
    await page.evaluate(mode => window.__setTaskTheme(mode), mode);
    await page.screenshot({path: resolve(output, `${mode}-1280-dense.png`), fullPage: true});
  }
  await page.goto(route('long')); await page.locator('.task-row').waitFor();
  const compact = (await page.locator('.task-background').boundingBox()).height;
  assert.ok(compact < 80);
  await page.getByRole('button', {name: '展开任务背景', exact: true}).click();
  assert.ok((await page.locator('.task-background').boundingBox()).height > compact * 2);
  await page.getByRole('button', {name: '收起任务背景', exact: true}).click();
  assert.ok((await page.locator('.goal').boundingBox()).height < 85);

  const app = await browser.newPage({viewport: {width: 1143, height: 949}, deviceScaleFactor: 2});
  app.on('pageerror', e => errors.push(e.message));
  for (const mode of ['light', 'dark']) {
    await app.goto(`http://127.0.0.1:1438/frontend/qa/main-window.html?theme=${mode}`);
    await app.getByRole('tab', {name: '任务与验收', exact: true}).click(); await app.locator('.task-row').waitFor();
    assert.equal(await app.locator('.content-pane>.content-navigation').count(), 1);
    await app.screenshot({path: resolve(output, `app-shell-${mode}-1143.png`)});
    await app.setViewportSize({width: 1143, height: 520});
    const top = (await app.locator('.content-navigation').boundingBox()).y;
    await app.locator('.task-row').first().click();
    const infoTop=(await app.locator('.task-info').boundingBox()).y;
    assert.ok(await app.locator('.task-run-scroll').evaluate(e => { e.scrollTop = e.scrollHeight; return e.scrollTop; }) > 0,'Execution body scrolls in short windows');
    assert.equal((await app.locator('.task-info').boundingBox()).y,infoTop);
    assert.equal(await app.locator('.content').evaluate(e=>e.scrollTop),0);
    assert.equal((await app.locator('.content-navigation').boundingBox()).y, top);
    await app.setViewportSize({width: 1143, height: 949});
  }
  await app.goto('http://127.0.0.1:1438/frontend/qa/main-window.html?taskOverflow=1');
  await app.getByRole('tab',{name:'任务与验收',exact:true}).click(); await app.locator('.task-row').nth(5).waitFor();
  assert.equal(await app.locator('.content').evaluate(e=>parseFloat(getComputedStyle(e).paddingTop)),12);
  assert.ok((await app.getByLabel('搜索任务',{exact:true}).boundingBox()).width<=281);
  assert.ok(await app.locator('.task-detail').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
  const scrollStyle=await app.locator('.content').evaluate(e=>({width:getComputedStyle(e,'::-webkit-scrollbar').width,color:getComputedStyle(e,'::-webkit-scrollbar-thumb').backgroundColor}));
  assert.ok(parseFloat(scrollStyle.width)<=6); assert.notEqual(scrollStyle.color,'rgb(128, 128, 128)');
  await app.screenshot({path:resolve(output,'compact-app-shell.png')});
  const visual = await browser.newPage({viewport: {width: 579, height: 760}, deviceScaleFactor: 2});
  await visual.goto(`${route('design')}&detailOnly=1`); await visual.locator('.run-card').nth(2).waitFor();
  await visual.screenshot({path: resolve(output, 'approved-detail-visual.png'), fullPage: true});
  await writeFile(resolve(output, 'report.json'), JSON.stringify({success: true, engine, pageErrors: errors, singleItemResult: true, editablePrompt: true, promptConflicts: true, runCardNavigation: true, themes: ['light', 'dark'], widths: [1280, 820, 600, 390]}, null, 2));
  assert.deepEqual(errors, []);
  await page.goto(route('local-verification')); await page.locator('.task-row').waitFor();
  const checks = page.locator('.criterion');
  assert.equal(await page.locator('.check-mark.passed').count(), await checks.count(), 'Local self-tests have visible status before release');
  await checks.first().locator(':scope>summary').click();
  await checks.first().getByRole('button',{name:'记录结果',exact:true}).click();
  const resultDialog=page.getByRole('dialog',{name:'记录验收结果',exact:true});
  assert.equal(await page.getByLabel('验收状态',{exact:true}).inputValue(),'');
  assert.ok(!(await resultDialog.locator('form').evaluate(form=>form.checkValidity())), 'Status must be explicitly selected with a conclusion');
  await page.keyboard.press('Escape');
  for (const empty of [false,true]) {
    await page.goto(route('owned-files'+(empty?'&emptyFiles=1':'')));await page.locator('.task-row').waitFor();
    await page.getByRole('button',{name:'查看变更',exact:true}).click();
    if(empty)await page.getByText('本次执行没有代码变更',{exact:true}).waitFor();
    else await page.locator('.change-file[title="frontend/feature.svelte"]').waitFor();
    await page.locator('.commit-changes>summary').first().click();
    await page.getByText('committed-change.ts',{exact:true}).waitFor();
    await closeChanges();
  }
  console.log(`Task UI passed (${engine}): run cards, four groups, per-item results, prompt editing/reset/conflict, focus, themes, responsive layout and fixed navigation.`);
} finally { await browser.close(); await server.close(); }
