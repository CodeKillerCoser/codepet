import {chromium,webkit} from 'playwright';
import {createServer} from 'vite';import {mkdir} from 'node:fs/promises';import {resolve} from 'node:path';import assert from 'node:assert/strict';
const engine=process.env.CODEPET_QA_ENGINE??'chromium';assert.ok(['chromium','webkit'].includes(engine));
const output=resolve('artifacts/task-ledger-qa/layout',engine==='webkit'?'webkit':'.');await mkdir(output,{recursive:true});
const server=await createServer({server:{host:'127.0.0.1',port:1438,strictPort:true}});await server.listen();
const browser=await ({chromium,webkit})[engine].launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1280,height:920}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html');await page.locator('.task-row').waitFor();
 assert.equal(await page.locator('.workbench .content-navigation').count(),0);
 const controls=await Promise.all(['[aria-label="搜索任务"]','[aria-label="任务项目"]','[aria-label="任务状态"]'].map(s=>page.locator(s).boundingBox()));assert.ok(Math.max(...controls.map(b=>b.y))-Math.min(...controls.map(b=>b.y))<2);
 assert.ok((await page.locator('.task-row').boundingBox()).height<100);assert.equal(await page.locator('.execution-records').getAttribute('open'),null,'Supporting facts start collapsed');
 assert.equal(await page.getByRole('region',{name:'任务执行',exact:true}).count(),1);assert.equal(await page.getByRole('region',{name:'验收清单',exact:true}).count(),1);assert.equal(await page.locator('.progress-line,.detail-objective,.delivery-hint').count(),0,'No duplicate goal or independent progress blocks');
 await page.locator('.execution-records>summary').click();assert.ok(await page.locator('.changed-file').count()>0,'Native snapshot contains observed files');await page.locator('.execution-records>summary').click();
 const history=page.getByLabel('任务执行历史',{exact:true}),oldId=await history.locator('option').nth(1).getAttribute('value');await history.selectOption(oldId);await page.locator('.closure').waitFor();assert.equal(await page.getByRole('button',{name:'更新执行',exact:true}).count(),0);await history.selectOption('');
 await page.getByLabel('更多任务操作',{exact:true}).click();await page.getByRole('button',{name:'接入 Agent',exact:true}).click();await page.getByRole('dialog').waitFor();assert.ok((await page.getByRole('dialog').innerText()).includes('X-CodePet-Agent'));await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'detached'});assert.ok(await page.getByLabel('更多任务操作').evaluate(e=>e===document.activeElement));
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  for(const width of [1280,820,600,390]){
   await page.setViewportSize({width,height:920});const back=page.getByRole('button',{name:'← 返回任务列表',exact:true});if(await back.isVisible())await back.click();
   await page.screenshot({path:resolve(output,`${mode}-${width}-list.png`),fullPage:true});assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   await page.locator('.task-row').click();assert.ok(await page.locator('.task-heading h3').evaluate(e=>e===document.activeElement));await page.screenshot({path:resolve(output,`${mode}-${width}-detail.png`),fullPage:true});assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   await page.getByRole('button',{name:'更新执行',exact:true}).click();const dialog=page.getByRole('dialog',{name:'更新执行',exact:true});await dialog.waitFor();assert.equal(await dialog.evaluate(e=>getComputedStyle(e).colorScheme),mode);await page.getByLabel('处理摘要',{exact:true}).focus();await page.screenshot({path:resolve(output,`${mode}-${width}-editor.png`)});await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});assert.ok(await page.getByRole('button',{name:'更新执行',exact:true}).evaluate(e=>e===document.activeElement));
   if(width<=600){assert.ok(await page.locator('.task-list').isHidden());await back.click();assert.ok(await page.locator('.task-row').evaluate(e=>e===document.activeElement));}
  }
 }
 await page.setViewportSize({width:1280,height:920});
 await page.getByRole('button',{name:'修改任务信息',exact:true}).click();await page.getByRole('dialog').waitFor();assert.equal(await page.getByLabel('处理摘要',{exact:true}).count(),0,'Task editor does not edit run data');await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'detached'});
 for(const name of ['记录人工验收','结束执行']){const trigger=page.getByRole('button',{name,exact:true});await trigger.click();const dialog=page.getByRole('dialog',{name,exact:true});await dialog.waitFor();assert.ok(await dialog.evaluate(e=>e.parentElement.classList.contains('app-window-host')));await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});assert.ok(await trigger.evaluate(e=>e===document.activeElement));}
 await page.evaluate(()=>{window.__holdTaskSave=true;});await page.getByRole('button',{name:'更新执行',exact:true}).click();await page.getByRole('button',{name:'保存执行',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.form-controls')?.disabled);await page.keyboard.press('Escape');assert.ok(await page.getByRole('dialog').isVisible());assert.ok(await page.getByRole('button',{name:'关闭执行编辑',exact:true}).isDisabled());await page.evaluate(()=>window.__rejectTaskSave());await page.waitForFunction(()=>!document.querySelector('.form-controls')?.disabled);await page.getByRole('button',{name:'关闭执行编辑',exact:true}).click();await page.getByRole('dialog').waitFor({state:'detached'});
 await page.getByLabel('搜索任务',{exact:true}).fill('升级 1.3.0');assert.equal(await page.locator('.task-row').count(),1,'Search includes earlier execution activity');await page.getByLabel('搜索任务').fill('不存在的任务');await page.getByText('没有符合条件的任务',{exact:true}).waitFor();
 await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html?scenario=single');await page.locator('.task-row').waitFor();assert.equal(await page.getByLabel('任务执行历史').count(),0);const tabs=page.getByRole('tablist',{name:'工作台视图',exact:true});await tabs.getByRole('tab',{name:'任务与验收',exact:true}).focus();await page.keyboard.press('ArrowLeft');await page.getByRole('button',{name:'确认',exact:true}).waitFor();await page.keyboard.press('ArrowRight');await page.locator('.task-row').waitFor();
 await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html?scenario=dense');await page.locator('.task-row').nth(7).waitFor();assert.equal(await page.locator('.criterion').count(),3);assert.equal(await page.locator('.criterion[open]').count(),1);assert.equal(await page.locator('.criterion').nth(1).getAttribute('open'),'');
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  const badges=await page.locator('.task-row .status-badge').evaluateAll(elements=>{const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const context=canvas.getContext('2d');const rgb=color=>{context.fillStyle=color;context.fillRect(0,0,1,1);return [...context.getImageData(0,0,1,1).data].slice(0,3);};return elements.map(e=>{const s=getComputedStyle(e);return {status:e.dataset.status,bg:s.backgroundColor,a:rgb(s.backgroundColor),b:rgb(s.color)};});});
  assert.equal(new Set(badges.map(b=>b.bg)).size,4);const lum=channels=>{const c=channels.map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return c[0]*.2126+c[1]*.7152+c[2]*.0722;};for(const badge of badges){const [a,b]=[lum(badge.a),lum(badge.b)].sort((a,b)=>b-a);assert.ok((a+.05)/(b+.05)>=4.5,`${mode}/${badge.status} contrast`);}await page.screenshot({path:resolve(output,`${mode}-1280-dense.png`),fullPage:true});
 }
 await page.goto('http://127.0.0.1:1438/frontend/qa/task-ledger.html?scenario=long');await page.locator('.task-row').waitFor();
 const compact=(await page.locator('.task-background').boundingBox()).height;assert.ok(compact<80,'Long background stays compact');
 await page.getByRole('button',{name:'展开任务背景',exact:true}).click();assert.ok((await page.locator('.task-background').boundingBox()).height>compact*2);await page.getByRole('button',{name:'收起任务背景',exact:true}).click();
 assert.ok((await page.locator('.run-description').boundingBox()).height<85,'Long execution summary stays compact');
 const app=await browser.newPage({viewport:{width:1143,height:949},deviceScaleFactor:2});app.on('pageerror',e=>errors.push(e.message));
 for(const mode of ['light','dark']){
  await app.goto(`http://127.0.0.1:1438/frontend/qa/main-window.html?theme=${mode}`);await app.getByRole('tab',{name:'任务与验收',exact:true}).click();await app.locator('.task-row').waitFor();assert.equal(await app.locator('.content-pane>.content-navigation').count(),1);await app.screenshot({path:resolve(output,`app-shell-${mode}-1143.png`)});
  await app.setViewportSize({width:1143,height:520});const top=(await app.locator('.content-navigation').boundingBox()).y;assert.ok(await app.locator('.content').evaluate(e=>{e.scrollTop=e.scrollHeight;return e.scrollTop;})>0);assert.equal((await app.locator('.content-navigation').boundingBox()).y,top);await app.setViewportSize({width:1143,height:949});
  await app.getByRole('button',{name:'设置',exact:true}).click();assert.equal(await app.locator('.content-navigation').count(),0);await app.getByRole('button',{name:'返回主界面',exact:true}).click();await app.locator('.task-row').waitFor();
  await app.getByRole('navigation',{name:'主导航',exact:true}).getByRole('button',{name:'连接接入',exact:true}).click();await app.getByRole('tablist',{name:'连接接入分区'}).waitFor();assert.equal(await app.locator('.content-pane>.content-navigation').count(),1);
 }
 assert.deepEqual(errors,[]);console.log(`Task/run UI passed (${engine}): two levels, independent editors, history, files, Header setup, busy/focus, themes, responsive layout and fixed App navigation.`);
}finally{await browser.close();await server.close();}
