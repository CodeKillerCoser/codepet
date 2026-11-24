import { chromium, webkit } from 'playwright';
import { createServer } from 'vite';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const engine=process.env.CODEPET_QA_ENGINE??'chromium';assert.ok(['chromium','webkit'].includes(engine));
const output=resolve('artifacts/task-ledger-qa/layout',engine==='webkit'?'webkit':'.');await mkdir(output,{recursive:true});
const server=await createServer({server:{host:'127.0.0.1',port:1438,strictPort:true}});await server.listen();
const browser=await ({chromium,webkit})[engine].launch({headless:true});
try{
  const page=await browser.newPage({viewport:{width:1280,height:920}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html');await page.locator('.task-row').waitFor();
  assert.equal(await page.locator('.workbench .content-navigation').count(),0,'Navigation is owned by the content container');
  const controls=await Promise.all(['[aria-label="搜索任务"]','[aria-label="任务项目"]','[aria-label="任务状态"]'].map(selector=>page.locator(selector).boundingBox()));
  assert.ok(Math.max(...controls.map(box=>box.y))-Math.min(...controls.map(box=>box.y))<2,'Desktop filters share the top toolbar');
  assert.equal(await page.locator('.task-list .filters').count(),0,'Filters no longer consume list space');
  assert.equal(await page.locator('.detail-top,.delivery-hint,.row-progress,.ledger-count').count(),0,'Redundant headings and delivery hints are removed');
  assert.match(await page.locator('.row-context').innerText(),/2 次处理/);
  const history=page.getByLabel('处理轮次',{exact:true});
  const historicalId=await history.locator('option').nth(1).getAttribute('value');
  await history.selectOption(historicalId);await page.locator('.closure').waitFor();
  assert.equal(await page.getByRole('button',{name:'编辑与记录进展',exact:true}).count(),0,'Past handling remains read-only');
  await history.selectOption('');await page.getByRole('button',{name:'编辑与记录进展',exact:true}).waitFor();
  assert.ok(await page.getByRole('button',{name:'接入 Codex',exact:true}).isHidden(),'MCP setup stays out of the primary toolbar');
  await page.getByLabel('更多任务操作',{exact:true}).click();await page.getByRole('button',{name:'接入 Codex',exact:true}).click();
  await page.getByRole('dialog').waitFor();await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'detached'});
  assert.ok(await page.getByLabel('更多任务操作',{exact:true}).evaluate(e=>e===document.activeElement),'Setup returns focus to its visible menu trigger');
  await page.getByLabel('更多任务操作',{exact:true}).focus();await page.keyboard.press('Enter');
  assert.ok(await page.getByRole('button',{name:'接入 Codex',exact:true}).isVisible());
  await page.keyboard.press('Escape');assert.ok(await page.getByRole('button',{name:'接入 Codex',exact:true}).isHidden());
  assert.ok((await page.locator('.task-row').boundingBox()).height<=100,'Task rows stay compact');
  assert.ok((await page.getByRole('region',{name:'方案与验收',exact:true}).boundingBox()).y<400,'Acceptance remains above secondary Git facts');
  for(const selector of ['.code-section','.background','.verification-section','.sources-section','.timeline-section'])assert.equal(await page.locator(selector).getAttribute('open'),null,`${selector} starts collapsed`);
  await page.locator('.code-section>summary').click();await page.locator('.code-section .path').waitFor();assert.equal(await page.locator('.code-section').getAttribute('open'),'');await page.locator('.code-section>summary').click();
  await page.locator('.criterion>summary').first().focus();await page.keyboard.press('Enter');assert.equal(await page.locator('.criterion').first().getAttribute('open'),null);await page.keyboard.press('Enter');assert.equal(await page.locator('.criterion').first().getAttribute('open'),'');
  for(const mode of ['light','dark']){
    await page.evaluate(mode=>window.__setTaskTheme(mode),mode);const themeColors=await page.locator('.workbench').evaluate(e=>{const s=getComputedStyle(e);return {background:s.backgroundColor,text:s.color};});assert.notEqual(themeColors.background,themeColors.text);if(mode==='dark')assert.notEqual(themeColors.background,'rgb(255, 255, 255)');
    for(const width of [1280,820,600,390]){
      await page.setViewportSize({width,height:920});
      if(await page.getByRole('button',{name:'← 返回任务列表',exact:true}).isVisible())await page.getByRole('button',{name:'← 返回任务列表',exact:true}).click();
      await page.screenshot({path:resolve(output,`${mode}-${width}-list.png`),fullPage:true});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`No list overflow: ${mode}/${width}`);
      await page.locator('.task-row').click();await page.locator('.task-detail h3').waitFor();assert.ok(await page.locator('.task-detail h3').evaluate(e=>e===document.activeElement));
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`No detail overflow: ${mode}/${width}`);
      await page.screenshot({path:resolve(output,`${mode}-${width}-detail.png`),fullPage:true});
      await page.getByRole('button',{name:'编辑与记录进展',exact:true}).click();const dialog=page.getByRole('dialog');await dialog.waitFor();
      assert.equal(await dialog.evaluate(e=>getComputedStyle(e).colorScheme),mode);
      await page.getByLabel('标题',{exact:true}).focus();assert.ok(await page.getByLabel('标题',{exact:true}).evaluate(e=>e===document.activeElement));
      await page.screenshot({path:resolve(output,`${mode}-${width}-editor.png`)});
      await page.keyboard.press('Escape');assert.ok(await page.getByRole('button',{name:'编辑与记录进展',exact:true}).evaluate(e=>e===document.activeElement));
      if(width<=600){assert.ok(await page.locator('.task-list').isHidden());await page.getByRole('button',{name:'← 返回任务列表',exact:true}).click();assert.ok(await page.locator('.task-row').evaluate(e=>e===document.activeElement));}
    }
  }
  await page.setViewportSize({width:1280,height:920});
  for(const [label,title] of [['记录人工验收','记录人工验收'],['结束任务','结束任务'],['接入 Codex','连接 Codex 任务登记']]){
    if(label==='接入 Codex')await page.getByLabel('更多任务操作',{exact:true}).click();
    const trigger=page.getByRole('button',{name:label,exact:true});await trigger.click();
    const modal=page.getByRole('dialog',{name:title,exact:true});await modal.waitFor();
    await page.waitForFunction(()=>document.querySelector('[role="dialog"]')?.contains(document.activeElement));
    assert.ok(await modal.evaluate(e=>e.parentElement.classList.contains('app-window-host')&&e.contains(document.activeElement)),'Task modals use the global host and focus scope');
    await page.keyboard.press('Escape');await modal.waitFor({state:'detached'});
    const returnTarget=label==='接入 Codex'?page.getByLabel('更多任务操作',{exact:true}):trigger;
    assert.ok(await returnTarget.evaluate(e=>e===document.activeElement),`${label} restores focus to its visible trigger`);
  }
  await page.evaluate(()=>{window.__holdTaskSave=true;});
  await page.getByRole('button',{name:'编辑与记录进展',exact:true}).click();
  await page.getByRole('button',{name:'保存任务',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.form-controls')?.disabled);
  await page.keyboard.press('Escape');
  assert.ok(await page.getByRole('dialog').isVisible(),'A pending save keeps the modal open on Escape');
  assert.ok(await page.getByRole('button',{name:'关闭任务编辑',exact:true}).isDisabled());
  await page.evaluate(()=>window.__rejectTaskSave());
  await page.waitForFunction(()=>!document.querySelector('.form-controls')?.disabled);
  await page.getByRole('button',{name:'关闭任务编辑',exact:true}).click();
  await page.getByRole('dialog').waitFor({state:'detached'});
  await page.getByLabel('搜索任务',{exact:true}).fill('升级 1.3.0');assert.equal(await page.locator('.task-row').count(),1);await page.getByLabel('搜索任务',{exact:true}).fill('不存在的任务');await page.getByText('没有符合条件的任务',{exact:true}).waitFor();
  await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html?scenario=single');await page.locator('.task-row').waitFor();
  assert.equal(await page.getByLabel('处理轮次',{exact:true}).count(),0,'A single handling does not show a history selector');
  const tabs=page.getByRole('tablist',{name:'工作台视图',exact:true});
  await tabs.getByRole('tab',{name:'任务与验收',exact:true}).focus();await page.keyboard.press('ArrowLeft');await page.getByRole('button',{name:'确认',exact:true}).waitFor();
  await page.keyboard.press('ArrowRight');await page.locator('.task-row').waitFor();
  await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html?scenario=dense');await page.locator('.task-row').nth(7).waitFor();
  assert.ok((await page.locator('.task-row').first().boundingBox()).height<=100);assert.equal(await page.locator('.criterion').count(),3);assert.equal(await page.locator('.criterion[open]').count(),1);
  assert.equal(await page.locator('.criterion').nth(1).getAttribute('open'),'','Failed acceptance items are expanded first');
  await page.getByText('菜单立即出现，点击事件无遗漏',{exact:true}).waitFor();await page.locator('.criterion>summary').nth(1).click();assert.equal(await page.locator('.criterion').nth(1).getAttribute('open'),null);await page.locator('.criterion>summary').nth(1).click();
  for(const mode of ['light','dark']){
    await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
    const badges=await page.locator('.task-row .status-badge').evaluateAll(elements=>{
      // WebKit can return display-p3 instead of rgb(). Sample into sRGB so the
      // contrast check measures the rendered colors, independent of CSS syntax.
      const canvas=document.createElement('canvas');canvas.width=canvas.height=1;
      const context=canvas.getContext('2d');
      const rgb=color=>{context.fillStyle=color;context.fillRect(0,0,1,1);return [...context.getImageData(0,0,1,1).data].slice(0,3);};
      return elements.map(e=>{const style=getComputedStyle(e);return {status:e.getAttribute('data-status'),text:e.textContent,bg:style.backgroundColor,bgRgb:rgb(style.backgroundColor),textRgb:rgb(style.color),width:e.getBoundingClientRect().width};});
    });
    assert.deepEqual([...new Set(badges.map(b=>b.status))].sort(),['closed','in_progress','published','ready_for_delivery']);
    assert.equal(new Set(badges.map(b=>b.bg)).size,4,`Each status uses a distinct subtle background in ${mode}`);
    const luminance=channels=>{const rgb=channels.map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;};
    for(const badge of badges){const values=[luminance(badge.bgRgb),luminance(badge.textRgb)].sort((a,b)=>b-a);assert.ok((values[0]+.05)/(values[1]+.05)>=4.5,`${mode} ${badge.status} text contrast`);assert.ok(badge.width<100,'Status color stays within a small badge');}
    await page.screenshot({path:resolve(output,`${mode}-1280-dense.png`),fullPage:true});
  }
  const appPage=await browser.newPage({viewport:{width:1143,height:949},deviceScaleFactor:2});appPage.on('pageerror',e=>errors.push(e.message));
  for(const mode of ['light','dark']){
    await appPage.goto(`http://127.0.0.1:1438/frontend/qa/main-window.html?theme=${mode}`);
    await appPage.getByRole('tab',{name:'任务与验收',exact:true}).click();await appPage.locator('.task-row').waitFor();
    assert.equal(await appPage.locator('.content-pane>.content-navigation').count(),1,'The App container owns secondary navigation');
    assert.equal(await appPage.locator('.content .horizontal-tabs').count(),0,'Navigation stays outside page content');
    const boxes=await Promise.all(['[aria-label="搜索任务"]','[aria-label="任务项目"]','[aria-label="任务状态"]'].map(selector=>appPage.locator(selector).boundingBox()));
    assert.ok(Math.max(...boxes.map(box=>box.y))-Math.min(...boxes.map(box=>box.y))<2,'Filters share one row at the annotated desktop width');
    await appPage.screenshot({path:resolve(output,`app-shell-${mode}-1143.png`)});
    await appPage.setViewportSize({width:1143,height:520});
    const navigationTop=(await appPage.locator('.content-navigation').boundingBox()).y;
    const scrollTop=await appPage.locator('.content').evaluate(element=>{element.scrollTop=element.scrollHeight;return element.scrollTop;});
    assert.ok(scrollTop>0,'The short-window scenario actually scrolls the content');
    assert.equal((await appPage.locator('.content-navigation').boundingBox()).y,navigationTop,'Secondary navigation stays fixed while content scrolls');
    await appPage.setViewportSize({width:1143,height:949});
    await appPage.getByRole('button',{name:'设置',exact:true}).click();assert.equal(await appPage.locator('.content-navigation').count(),0,'Settings without secondary navigation reserve no slot');
    await appPage.getByRole('button',{name:'返回主界面',exact:true}).click();await appPage.locator('.task-row').waitFor();
    await appPage.getByRole('navigation',{name:'主导航',exact:true}).getByRole('button',{name:'连接接入',exact:true}).click();
    await appPage.getByRole('tablist',{name:'连接接入分区',exact:true}).waitFor();
    assert.equal(await appPage.locator('.content-pane>.content-navigation').count(),1,'Connection pages reuse the same container slot');
    await appPage.getByRole('navigation',{name:'主导航',exact:true}).getByRole('button',{name:'用量',exact:true}).click();
    assert.equal(await appPage.locator('.content-navigation').count(),0,'A page without secondary navigation reserves no space');
    await appPage.getByRole('navigation',{name:'主导航',exact:true}).getByRole('button',{name:'Codex工作台',exact:true}).click();
    await appPage.locator('.task-row').waitFor();assert.equal(await appPage.getByRole('tab',{name:'任务与验收',exact:true}).getAttribute('aria-selected'),'true','Returning preserves the selected workbench view');
  }
  assert.deepEqual(errors,[]);console.log('Task layout passed: shared App navigation, compact toolbar, handling history, MCP menu/keyboard/focus, real native snapshot, light/dark themes, desktop/narrow widths, modal/mobile focus and filtering.');
}finally{await browser.close();await server.close();}
