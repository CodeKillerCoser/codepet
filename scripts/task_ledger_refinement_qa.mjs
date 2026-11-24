import {chromium,webkit} from 'playwright';
import {createServer} from 'vite';
import {mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import assert from 'node:assert/strict';
const engine=process.env.CODEPET_QA_ENGINE??'chromium';
const output=resolve('artifacts/task-ledger-qa/refinement',engine);await mkdir(output,{recursive:true});
const server=await createServer({server:{host:'127.0.0.1',port:0,strictPort:true}});await server.listen();const port=server.httpServer.address().port;
const browser=await({chromium,webkit})[engine].launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1280,height:920}}),errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 const goto=async scenario=>{await page.goto(`http://127.0.0.1:${port}/frontend/qa/task-ledger.html?scenario=${scenario}`);await page.locator('.task-row').first().waitFor();};
 const close=async()=>{await page.getByRole('button',{name:'关闭代码变更',exact:true}).click();await page.locator('.app-side-panel').waitFor({state:'detached'});};
 await goto('dense');
 const handle=page.getByRole('separator',{name:'调整任务列表宽度',exact:true});
 const width=()=>page.locator('.task-list').evaluate(e=>Math.round(e.getBoundingClientRect().width));
 assert.equal(await width(),280);
 let box=await handle.boundingBox();await page.mouse.move(box.x+4,box.y+100);await page.mouse.down();await page.mouse.move(box.x+64,box.y+100);await page.mouse.up();
 assert.equal(await width(),340,'Pointer resizes the list and detail together');
 box=await handle.boundingBox();await page.mouse.move(box.x+4,box.y+100);await page.mouse.down();await page.mouse.move(box.x+60,box.y+100);await page.keyboard.press('Escape');await page.mouse.up();assert.equal(await width(),340,'Escape cancels drag');
 await handle.focus();await page.keyboard.press('Home');assert.equal(await width(),220);await page.keyboard.press('ArrowRight');assert.equal(await width(),236);await page.keyboard.press('End');assert.equal(await width(),440);
 await page.setViewportSize({width:820,height:920});await page.waitForFunction(()=>document.querySelector('.task-list').getBoundingClientRect().width<440);await page.setViewportSize({width:1280,height:920});await page.waitForFunction(()=>Math.round(document.querySelector('.task-list').getBoundingClientRect().width)===440);
 await handle.dblclick();assert.equal(await width(),280);
 await page.locator('.task-row').first().click();
 await page.setViewportSize({width:1280,height:620});
 const info=page.locator('.task-info'),body=page.locator('.task-run-scroll');
 const infoTop=(await info.boundingBox()).y,navTop=(await page.locator('.content-navigation').boundingBox()).y,filtersTop=(await page.locator('.toolbar').boundingBox()).y;
 await body.focus();await page.keyboard.press('PageDown');await page.waitForFunction(()=>document.querySelector('.task-run-scroll').scrollTop>0);
 assert.equal((await info.boundingBox()).y,infoTop);assert.equal((await page.locator('.content-navigation').boundingBox()).y,navTop);assert.equal((await page.locator('.toolbar').boundingBox()).y,filtersTop);
 assert.equal(await page.locator('.content').evaluate(e=>e.scrollTop),0);
 const edit=page.getByRole('button',{name:'修改任务信息',exact:true});assert.equal(await edit.innerText(),'');assert.equal(await edit.locator('svg').count(),1);
 await edit.click();await page.getByRole('dialog',{name:'修改任务信息',exact:true}).waitFor();await page.keyboard.press('Escape');
 assert.equal(await page.locator('button button').count(),0,'Cards never nest link buttons');
 assert.equal(await page.locator('.workspace-section .info-grid>div').count(),3,'Commit and changes share one row');
 assert.equal(await page.locator('.code-summary .code-state').count(),1);
 await page.setViewportSize({width:1280,height:920});await body.evaluate(e=>e.scrollTop=0);
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  const green=await page.locator('.check-mark.passed').first().evaluate(e=>{const expected=document.createElement('span');expected.style.color='var(--color-success)';e.append(expected);const same=getComputedStyle(e).color===getComputedStyle(expected).color;expected.remove();return same;});assert.ok(green,'Check uses semantic green in each theme');
  await page.screenshot({path:resolve(output,`fixed-header-${mode}.png`)});
 }
 await goto('cached-files');
 await page.locator('.code-summary').getByText('3 个文件',{exact:true}).waitFor();
 for(const label of ['新增 1 个文件','修改 1 个文件','删除 1 个文件'])assert.equal(await page.locator('.code-summary').getByLabel(label,{exact:true}).count(),1,'Cached snapshots project synchronously');
 await page.getByRole('button',{name:'查看变更',exact:true}).click();await page.locator('.change-file[title="cached-added.ts"]').waitFor();await close();
 await goto('design');await page.locator('.run-card').last().click();
 const lastId=await page.locator('.run-card').last().getAttribute('id');
 await page.locator('.run-card-shell').first().getByRole('button',{name:/打开 Codex 对话/}).click();
 assert.equal(await page.locator('.run-card[aria-selected="true"]').getAttribute('id'),lastId);
 assert.equal(await page.evaluate(()=>window.__openedTaskChat),'00000000-0000-4000-8000-000000000000');
 assert.equal(await page.locator('.run-content .conversation-link').count(),0,'Conversation links belong to their run cards');
 await page.setViewportSize({width:1440,height:920});
 await page.getByRole('button',{name:'查看变更',exact:true}).click();
 const panel=page.locator('.app-side-panel');await panel.waitFor();await page.waitForFunction(()=>!document.getAnimations().some(a=>a.playState==='running'));assert.equal(await panel.getAttribute('data-mode'),'column');
 assert.ok(await panel.evaluate(e=>!e.closest('.task-ledger')),'Panel is hosted by the app framework');
 await page.locator('.change-file[title="committed-change.ts"]').waitFor();assert.ok(await panel.getByLabel('修改 1 个文件',{exact:true}).count());
 const file=page.locator('.change-file').first();await file.locator('summary').click();
 await page.getByRole('textbox',{name:'只读代码差异',exact:true}).waitFor();
 assert.ok(await page.locator('.diff-line.added').count()>0);assert.ok(await page.locator('.diff-line.deleted').count()>0);
 assert.equal(await page.locator('.diff-line.added .line-number').last().innerText(),'2');
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  const readability=await page.locator('.info-grid').first().evaluate(e=>{
   const color=css=>{const c=document.createElement('canvas');c.width=c.height=1;const x=c.getContext('2d');x.fillStyle=css;x.fillRect(0,0,1,1);return [...x.getImageData(0,0,1,1).data].slice(0,3);};
   const luminance=rgb=>rgb.map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((a,v,i)=>a+v*[.2126,.7152,.0722][i],0);
   const probe=document.createElement('span');probe.style.color='var(--wb-panel)';e.append(probe);const bg=luminance(color(getComputedStyle(probe).color));probe.remove();const label=e.querySelector('dt'),text=luminance(color(getComputedStyle(label).color));
   return {size:parseFloat(getComputedStyle(e).fontSize),contrast:(Math.max(bg,text)+.05)/(Math.min(bg,text)+.05)};
  });
  assert.ok(readability.size>=14,'Task body is at least 14px');assert.ok(readability.contrast>=4.5,`Secondary text contrast ${mode}: ${readability.contrast}`);
  assert.equal(await panel.locator('.diff-lines').evaluate(e=>e.scrollWidth<=e.clientWidth),true,'Code wraps inside panel');
  await page.screenshot({path:resolve(output,`diff-${mode}.png`)});
 }
 const criterion=page.locator('.criterion').first();if(await criterion.getAttribute('open')===null)await criterion.locator(':scope>summary').click();
 assert.deepEqual(await criterion.locator('.criterion-body>div>dt').allTextContents(),['验收方式','预期结果','验收结果']);
 assert.equal(await page.locator('.criterion-history').count(),0,'No separate acceptance history block');
 await page.screenshot({path:resolve(output,'global-column.png')});
 await edit.click();await page.getByRole('dialog',{name:'修改任务信息',exact:true}).waitFor();await page.keyboard.press('Escape');assert.equal(await panel.count(),1);await close();
 assert.ok(await page.getByRole('button',{name:'查看变更',exact:true}).evaluate(e=>e===document.activeElement));
 await goto('overflow');await page.getByRole('button',{name:'查看变更',exact:true}).click();await panel.waitFor();await page.locator('.task-row').nth(1).click();await panel.waitFor({state:'detached'});
 assert.equal(await page.locator('.task-heading h3').innerText(),'统一全局侧栏卡片、动画与弹窗层级');
 for(const viewport of [{width:820,height:920},{width:390,height:760}]){
  await page.setViewportSize(viewport);await goto('design');await page.locator('.task-row').first().click();
  await page.getByRole('button',{name:'查看变更',exact:true}).click();await panel.waitFor();assert.equal(await panel.getAttribute('data-mode'),'drawer');
  await page.waitForFunction(()=>{const e=document.querySelector('.app-side-panel');return e&&e.getBoundingClientRect().right<=innerWidth+1&&!document.getAnimations().some(a=>a.playState==='running');});
  assert.ok(await panel.evaluate(e=>e.getBoundingClientRect().right<=innerWidth+1));assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:resolve(output,`global-drawer-${viewport.width}.png`)});await page.keyboard.press('Escape');await panel.waitFor({state:'detached'});
  assert.ok(await page.getByRole('button',{name:'查看变更',exact:true}).evaluate(e=>e===document.activeElement));
 }
 assert.deepEqual(errors,[]);console.log(`Task refinement passed (${engine}): pointer/keyboard/cancel/default widths, fixed information, card links, edit icon, green checks, global column/drawer, layered focus and row cleanup.`);
}finally{await browser.close();await server.close();}
