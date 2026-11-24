import {chromium,webkit} from 'playwright';
import {createServer} from 'vite';
import {mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import assert from 'node:assert/strict';
const engine=process.env.CODEPET_QA_ENGINE??'chromium';
const output=resolve('artifacts/task-ledger-qa/refinement',engine);await mkdir(output,{recursive:true});
const server=await createServer({cacheDir:resolve(`node_modules/.vite-task-refinement-${engine}`),server:{host:'127.0.0.1',port:1441,strictPort:true}});await server.listen();const port=server.httpServer.address().port;
const browser=await({chromium,webkit})[engine].launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1440,height:920}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 const goto=async scenario=>{await page.goto(`http://127.0.0.1:${port}/frontend/qa/task-ledger.html?scenario=${scenario}`);await page.locator('.task-row').first().waitFor();const id=await page.evaluate(()=>window.__taskCompletionState()[0].id);await page.locator(`[data-task-id="${id}"]`).click();await page.locator('.task-heading h3').waitFor();};
 const panel=page.locator('.app-side-panel');
 const open=async()=>{await page.locator('.run-card').first().click();await page.getByLabel('任务执行详情',{exact:true}).waitFor();};
 const close=async()=>{await page.getByRole('button',{name:'关闭执行详情',exact:true}).click();await panel.waitFor({state:'detached'});};
 await goto('dense');
 const title=await page.locator('.task-heading h3').innerText();
 await page.getByRole('button',{name:'返回任务列表',exact:true}).click();await page.getByRole('button',{name:'表格',exact:true}).click();
 await page.getByRole('tab',{name:'代码状态',exact:true}).click();await page.getByRole('tab',{name:'任务与验收',exact:true}).click();await page.getByRole('table').waitFor();
 assert.equal(await page.getByRole('button',{name:'表格',exact:true}).getAttribute('aria-pressed'),'true');
 await page.locator('.table-task').filter({hasText:title}).click();await open();
 assert.equal(await panel.getAttribute('data-mode'),'column');assert.ok(await panel.evaluate(e=>!e.closest('.task-ledger')));
 const edit=page.getByRole('button',{name:'修改任务信息',exact:true});await edit.click();await page.getByRole('dialog',{name:'修改任务信息',exact:true}).waitFor();await page.keyboard.press('Escape');assert.equal(await panel.count(),1,'Editing is layered over the framework panel');
 assert.equal(await panel.locator('.workspace-section .info-grid>div').count(),3);
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  const readability=await panel.locator('.info-grid').first().evaluate(e=>{
   const rgb=css=>{const x=document.createElement('canvas').getContext('2d');x.fillStyle=css;x.fillRect(0,0,1,1);return [...x.getImageData(0,0,1,1).data].slice(0,3);};
   const lum=rgb=>rgb.map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((a,v,i)=>a+v*[.2126,.7152,.0722][i],0);
   const probe=document.createElement('span');probe.style.color='var(--wb-panel)';e.append(probe);const bg=lum(rgb(getComputedStyle(probe).color));probe.remove();const text=lum(rgb(getComputedStyle(e.querySelector('dt')).color));return {size:parseFloat(getComputedStyle(e).fontSize),contrast:(Math.max(bg,text)+.05)/(Math.min(bg,text)+.05)};
  });
  assert.ok(readability.size>=15);assert.ok(readability.contrast>=4.5,`Text contrast ${mode}: ${readability.contrast}`);
  assert.ok(await panel.locator('.check-mark.passed').first().evaluate(e=>{const probe=document.createElement('span');probe.style.color='var(--color-success)';e.append(probe);const same=getComputedStyle(e).color===getComputedStyle(probe).color;probe.remove();return same;}));
  await page.screenshot({path:resolve(output,`overview-and-run-${mode}.png`)});
 }
 await panel.getByRole('button',{name:'查看变更',exact:true}).click();await panel.locator('.change-file>summary').first().click();await page.getByRole('textbox',{name:'只读代码差异',exact:true}).waitFor();
 assert.ok(await panel.locator('.diff-lines').evaluate(e=>e.scrollWidth<=e.clientWidth+1));assert.equal(await page.locator('.app-side-panel').count(),1);
 await panel.getByRole('button',{name:'返回执行详情',exact:true}).click();assert.ok(await panel.getByRole('button',{name:'查看变更',exact:true}).evaluate(e=>e===document.activeElement));
 await close();assert.ok(await page.locator('.run-card').first().evaluate(e=>e===document.activeElement));
 await open();await page.getByRole('button',{name:'返回任务列表',exact:true}).click();await panel.waitFor({state:'detached'});assert.equal(await page.locator('.task-heading').count(),0);
 await goto('cached-files');await open();await panel.locator('.code-summary').getByText('3 个文件',{exact:true}).waitFor();for(const label of ['新增 1 个文件','修改 1 个文件','删除 1 个文件'])assert.equal(await panel.locator('.code-summary').getByLabel(label,{exact:true}).count(),1);
 await close();
 for(const viewport of [{width:820,height:760},{width:390,height:760}]){
  await page.setViewportSize(viewport);await goto('design');await open();assert.equal(await panel.getAttribute('data-mode'),'drawer');
  await page.waitForFunction(()=>!document.getAnimations().some(a=>a.playState==='running'));
  assert.ok(await panel.evaluate(e=>e.getBoundingClientRect().right<=innerWidth+1));assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  const before=await page.locator('.task-info').boundingBox();await panel.locator('.panel-body').evaluate(e=>e.scrollTop=e.scrollHeight);assert.equal((await page.locator('.task-info').boundingBox()).y,before.y);
  await page.screenshot({path:resolve(output,`run-drawer-${viewport.width}.png`)});await page.keyboard.press('Escape');await panel.waitFor({state:'detached'});assert.ok(await page.locator('.run-card').first().evaluate(e=>e===document.activeElement));
 }
 const app=await browser.newPage({viewport:{width:1440,height:760}});app.on('pageerror',e=>errors.push(e.message));
 await app.goto(`http://127.0.0.1:${port}/frontend/qa/main-window.html?taskOverflow=1`);await app.getByRole('tab',{name:'任务与验收',exact:true}).click();await app.locator('.task-row').nth(5).waitFor();
 await app.locator('.task-row').first().click();await app.locator('.run-card').first().click();await app.locator('.task-run-panel').waitFor();
 const infoTop=(await app.locator('.task-info').boundingBox()).y,navTop=(await app.locator('.content-navigation').boundingBox()).y;
 await app.locator('.panel-body').evaluate(e=>e.scrollTop=e.scrollHeight);assert.equal((await app.locator('.task-info').boundingBox()).y,infoTop);assert.equal((await app.locator('.content-navigation').boundingBox()).y,navTop);assert.equal(await app.locator('.content').evaluate(e=>e.scrollTop),0);
 await app.screenshot({path:resolve(output,'actual-shell-run-panel.png')});
 assert.deepEqual(errors,[]);console.log(`Task refinement passed (${engine}): Task collection/view persistence, accessible focus, 15px contrast, one framework panel, diff return, switching cleanup and fixed app navigation.`);
}finally{await browser.close();await server.close();}
