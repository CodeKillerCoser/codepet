import {chromium,webkit} from 'playwright';
import {createServer} from 'vite';
import {mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import assert from 'node:assert/strict';

const engine=process.env.CODEPET_QA_ENGINE??'chromium';
assert.ok(['chromium','webkit'].includes(engine));
const output=resolve('artifacts/task-ledger-qa/layout',engine);await mkdir(output,{recursive:true});
const server=await createServer({cacheDir:resolve(`node_modules/.vite-task-ui-${engine}`),server:{watch:{ignored:['**/src-tauri/target/**']},host:'127.0.0.1',port:0}});await server.listen();
const port=server.httpServer.address().port;
const browser=await({chromium,webkit})[engine].launch({headless:true});
const errors=[];
try{
 const page=await browser.newPage({viewport:{width:1440,height:920}});page.on('pageerror',e=>errors.push(e.message));
 const route=scenario=>`http://127.0.0.1:${port}/frontend/qa/task-ledger.html${scenario?`?scenario=${scenario}`:''}`;
 const goto=async(scenario,open=true)=>{await page.goto(route(scenario));await page.locator('.task-row').first().waitFor();if(open){const id=await page.evaluate(()=>window.__taskCompletionState()[0].id);await page.locator(`[data-task-id="${id}"]`).click();await page.locator('.task-heading h3').waitFor();}};
 const panel=page.locator('.app-side-panel');
 const openRun=async(index=0)=>{await page.locator('.run-card').nth(index).click();await page.getByLabel('任务执行详情',{exact:true}).waitFor();};
 const close=async()=>{await page.getByRole('button',{name:'关闭执行详情',exact:true}).click();await panel.waitFor({state:'detached'});};
 await goto('dense',false);
 assert.equal(await page.locator('.board-task').count(),8,'A card is a Task, never an individual TaskRun');
 assert.deepEqual(await page.locator('.column-count').allTextContents(),['2','2','2','2'],'Grouping uses the backend Task summary');
 const assertSingleRow=async()=>{
  const tops=await page.locator('.board-column').evaluateAll(es=>es.map(e=>e.getBoundingClientRect().top));
  assert.equal(tops.length,4);assert.ok(tops.every(top=>Math.abs(top-tops[0])<1),'All four states stay on one row');
 };
 await assertSingleRow();
 // The available content width matters, including shell navigation/panel changes.
 await page.locator('.task-ledger').evaluate(e=>e.style.width='959px');
 await page.getByRole('table').waitFor();assert.ok(await page.getByRole('button',{name:'看板',exact:true}).isDisabled());
 await page.getByLabel('搜索任务',{exact:true}).fill('菜单');
 await page.locator('.task-ledger').evaluate(e=>e.style.width='960px');
 await page.locator('.task-board').waitFor();await assertSingleRow();
 assert.equal(await page.getByLabel('搜索任务',{exact:true}).inputValue(),'菜单','Auto switching preserves filters');
 await page.getByLabel('搜索任务',{exact:true}).fill('');
 await page.locator('.task-ledger').evaluate(e=>e.style.removeProperty('width'));
 await page.getByRole('button',{name:'表格',exact:true}).click();await page.getByRole('table').waitFor();
 assert.equal(await page.locator('.task-table tbody tr').count(),8);
 assert.equal(await page.locator('.task-table tbody tr').first().locator('.runs-column').innerText(),'2');
 await page.getByRole('button',{name:'任务',exact:true}).click();
 const sortedTitles=await page.locator('.table-task').allTextContents();assert.deepEqual(sortedTitles,[...sortedTitles].sort((a,b)=>a.localeCompare(b,'zh-CN')));
 await page.getByLabel('任务状态',{exact:true}).selectOption('closed');assert.equal(await page.locator('.table-task').count(),2);
 await page.getByRole('button',{name:'看板',exact:true}).click();assert.equal(await page.locator('.board-column').count(),1);assert.equal(await page.locator('.board-task').count(),2);
 await page.getByRole('tab',{name:'代码状态',exact:true}).click();await page.getByRole('tab',{name:'任务与验收',exact:true}).click();await page.locator('.board-task').first().waitFor();assert.equal(await page.getByLabel('任务状态',{exact:true}).inputValue(),'closed','Filters and view survive page switches');
 await page.getByLabel('任务状态',{exact:true}).selectOption('all');
 await page.getByLabel('搜索任务',{exact:true}).fill('不存在的任务');await page.getByRole('heading',{name:'没有符合条件的任务',exact:true}).waitFor();await page.getByLabel('搜索任务',{exact:true}).fill('');
 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  for(const width of [1440,1280,820,390]){
   await page.setViewportSize({width,height:860});
   const fits=await page.locator('.task-ledger').evaluate(e=>e.clientWidth>=960);
   await page.waitForFunction(disabled=>[...document.querySelectorAll('.view-switch button')].find(e=>e.textContent.trim()==='看板')?.disabled===disabled,!fits);
   if(!fits)await page.locator('.task-table').waitFor();
   assert.equal(await page.getByRole('button',{name:'看板',exact:true}).isDisabled(),!fits);
   for(const name of fits?['看板','表格']:['表格']){
    await page.getByRole('button',{name,exact:true}).click();
    if(name==='看板')await assertSingleRow();
    assert.equal(await page.locator('.task-row').count(),8);
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Neither view overflows the main page');
    assert.ok(await page.locator('.task-collection').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
    await page.screenshot({path:resolve(output,`collection-${mode}-${width}-${name==='看板'?'board':'table'}.png`)});
   }
  }
 }
 await page.setViewportSize({width:1440,height:920});
 await page.locator('.task-board').waitFor();
 await page.getByRole('button',{name:'表格',exact:true}).click();
 await page.locator('.table-task').first().focus();await page.keyboard.press('Enter');await page.locator('.task-heading h3').waitFor();
 await page.getByRole('button',{name:'返回任务列表',exact:true}).click();assert.ok(await page.locator('.table-task').first().evaluate(e=>e===document.activeElement),'Returning focuses the correct task in the same view');
 await goto('design');
 assert.equal(await page.locator('.run-card').count(),3);
 assert.equal(await page.locator('.run-content').count(),0,'Run details load only on explicit selection');
 assert.equal(await panel.count(),0);
 assert.equal(await page.locator('.run-title strong').first().innerText(),'修复重启丢失','Latest execution first');
 const positions=await page.locator('.task-heading,.task-background,.task-context,.task-divider,.run-navigation').evaluateAll(es=>es.map(e=>e.getBoundingClientRect().top));
 assert.ok(positions.every((y,i)=>!i||y>positions[i-1]),'Task title, description, metadata and execution list keep hierarchy');
 await openRun();
 for(const name of ['目标与方案','工作区','验收清单','活动时间线'])assert.equal(await panel.getByRole('region',{name,exact:true}).count(),1);
 assert.ok(await panel.evaluate(e=>!e.closest('.task-ledger')),'Uses the framework panel');
 assert.equal(await page.locator('button button').count(),0);
 assert.equal(await panel.getByRole('button',{name:'执行状态：待确认',exact:true}).count(),1);
 await panel.getByRole('button',{name:/打开 Codex 对话/}).click();
 assert.equal(await page.evaluate(()=>window.__openedTaskChat),'00000000-0000-4000-8000-000000000000');
 await panel.getByRole('button',{name:'查看变更',exact:true}).click();
 await panel.locator('.change-file').first().waitFor();assert.equal(await page.locator('.app-side-panel').count(),1,'No second private drawer');
 await panel.locator('.change-file>summary').first().click();await page.getByRole('textbox',{name:'只读代码差异',exact:true}).waitFor();
 assert.ok(await panel.locator('.diff-line.added').count()>0);assert.ok(await panel.locator('.diff-line.deleted').count()>0);
 await panel.getByRole('button',{name:'返回执行详情',exact:true}).click();
 assert.ok(await panel.getByRole('button',{name:'查看变更',exact:true}).evaluate(e=>e===document.activeElement));
 await close();assert.ok(await page.locator('.run-card').first().evaluate(e=>e===document.activeElement));
 await openRun(2);assert.equal(await panel.getByRole('button',{name:'执行状态：待确认',exact:true}).count(),0,'History stays read-only');await close();
 await page.locator('.run-card').first().focus();await page.keyboard.press('End');assert.ok(await page.locator('.run-card').last().evaluate(e=>e===document.activeElement));await page.keyboard.press('Home');assert.ok(await page.locator('.run-card').first().evaluate(e=>e===document.activeElement));

 for(const mode of ['light','dark']){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);
  for(const width of [1440,1280,820,600,390]){
   await page.setViewportSize({width,height:820});await page.waitForTimeout(250);
   await page.getByRole('button',{name:'返回任务列表',exact:true}).click();
   await page.locator('.task-row').first().click();
   assert.ok(await page.locator('.task-heading h3').evaluate(e=>e===document.activeElement));
   await page.screenshot({path:resolve(output,`${mode}-${width}-overview.png`)});
   await openRun();
   assert.equal(await panel.getAttribute('data-mode'),width>=1180?'column':'drawer');
   const criterion=panel.locator('.criterion').first();if(await criterion.getAttribute('open')===null)await criterion.locator('summary').click();
   assert.deepEqual(await criterion.locator('.criterion-body>div>dt').allTextContents(),['验收方式','预期结果','验收结果']);
   assert.equal(await criterion.locator('.criterion-result,.record-result,.criterion-history').count(),0);
   await criterion.locator('summary').focus();await page.keyboard.press('Enter');assert.equal(await criterion.getAttribute('open'),null);
   assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   assert.ok(await panel.locator('.panel-body').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
   assert.ok(await panel.locator('.info-grid').first().evaluate(e=>parseFloat(getComputedStyle(e).fontSize)>=15));
   await page.screenshot({path:resolve(output,`${mode}-${width}-run-panel.png`)});
   await page.keyboard.press('Escape');await panel.waitFor({state:'detached'});
   assert.ok(await page.locator('.run-card').first().evaluate(e=>e===document.activeElement));
  }
 }
 await page.setViewportSize({width:1440,height:920});await goto('overflow&manyRuns=1');await page.locator('.run-card').nth(11).waitFor();
 assert.ok(await page.locator('.run-navigation').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
 await page.locator('.task-run-scroll').focus();await page.keyboard.press('PageDown');await page.waitForFunction(()=>document.querySelector('.task-run-scroll').scrollTop>0);
 const infoTop=(await page.locator('.task-info').boundingBox()).y;
 await openRun();assert.equal((await page.locator('.task-info').boundingBox()).y,infoTop);
 await close();
 await goto('long');const compact=(await page.locator('.task-background').boundingBox()).height;
 await page.getByRole('button',{name:'展开任务背景',exact:true}).click();assert.ok((await page.locator('.task-background').boundingBox()).height>compact*2);
 await page.getByRole('button',{name:'收起任务背景',exact:true}).click();
 await openRun();assert.ok((await page.locator('.goal').boundingBox()).height<90);await panel.getByRole('button',{name:'展开目标',exact:true}).click();assert.ok((await page.locator('.goal').boundingBox()).height>90);await close();

 // Metadata and editable Agent instructions remain available on the overview.
 await page.getByRole('button',{name:'修改任务信息',exact:true}).click();await page.getByRole('dialog',{name:'修改任务信息',exact:true}).waitFor();
 await page.getByLabel('背景与现象').fill('保留未保存的背景草稿');await page.evaluate(()=>window.__holdTaskSave=true);await page.getByRole('button',{name:'确认',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.form-controls')?.disabled);
 await page.keyboard.press('Escape');assert.ok(await page.getByRole('button',{name:'关闭任务编辑',exact:true}).isDisabled());
 await page.evaluate(()=>{window.__holdTaskSave=false;window.__rejectTaskSave();});await page.waitForFunction(()=>!document.querySelector('.form-controls')?.disabled);assert.equal(await page.getByLabel('背景与现象').inputValue(),'保留未保存的背景草稿');await page.keyboard.press('Escape');
 await page.getByLabel('更多任务操作').click();await page.getByRole('button',{name:'接入 Agent',exact:true}).click();
 await page.evaluate(()=>{window.__copiedAgentConfig='';Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__copiedAgentConfig=text;}}});});
 const setupDialog=page.getByRole('dialog',{name:'连接 Agent',exact:true});
 assert.equal(await setupDialog.locator('.agent-clients button').count(),5);
 const setupPrompt=setupDialog.getByRole('textbox',{name:'扩展提示词',exact:true});
 assert.equal(await setupPrompt.isVisible(),true,'Extension prompt is directly editable');
 assert.equal(await setupDialog.locator('.prompt-editor summary').count(),0,'No duplicate editing title');
 assert.equal(await setupDialog.locator('.prompt-editor footer').evaluate(e=>getComputedStyle(e).borderTopWidth),'0px','No divider above prompt actions');
 assert.equal(await setupDialog.locator('.config-destination,.usage-guidance,.config-code').count(),0,'No standalone configuration or guidance blocks');
 const edited='当前编辑的约定：实施前登记，完成前提交结论。';
 const original=await setupPrompt.inputValue();await setupPrompt.fill(edited);
 for(const name of ['Codex','Claude Code','Cursor','OpenCode','Antigravity']){
  const button=setupDialog.getByRole('button',{name:`复制 ${name} 安装指令`,exact:true});
  const tooltip=setupDialog.getByRole('tooltip',{name,exact:true});
  assert.equal(await tooltip.isVisible(),false);
  await button.hover();await tooltip.waitFor({state:'visible'});
  assert.equal(await button.innerText(),'','Only the Agent mark is visible by default');
  await button.click();
  const copied=await page.evaluate(()=>window.__copiedAgentConfig);
  assert.ok(copied.includes('安装这个 MCP')&&copied.includes('系统提示词'));
  assert.ok(copied.includes(edited),'Copies the current draft, even before saving');
  if(name==='Codex')assert.ok(copied.includes('[mcp_servers.codepet_tasks]'));
  else{const config=JSON.parse(copied.match(/```json\n([\s\S]*?)\n```/)[1]),id={'Claude Code':'claude',Cursor:'cursor',OpenCode:'opencode',Antigravity:'antigravity'}[name];
   assert.equal(name==='OpenCode'?config.mcp.servers.codepet_tasks.environment.CODEPET_AGENT:config.mcpServers.codepet_tasks.env.CODEPET_AGENT,id);
   if(name==='Antigravity'){assert.ok(copied.includes('~/.gemini/config/mcp_config.json'));assert.ok(copied.includes('~/.gemini/GEMINI.md'));}
  }
  assert.equal(await setupDialog.getByRole('status').innerText(),'已复制');
  await page.getByRole('heading',{name:'连接 Agent',exact:true}).last().hover();
 }
 await setupPrompt.fill(original);
 await page.evaluate(()=>{navigator.clipboard.writeText=async()=>{throw Error('blocked');};});
 await setupDialog.getByRole('button',{name:'复制 Codex 安装指令',exact:true}).click();
 await setupDialog.getByRole('alert').filter({hasText:'复制失败'}).waitFor();
 await page.evaluate(()=>{navigator.clipboard.writeText=async text=>{window.__copiedAgentConfig=text;};});
 await setupDialog.getByRole('button',{name:'复制 Codex 安装指令',exact:true}).focus();await page.keyboard.press('Enter');
 await setupDialog.getByRole('status').filter({hasText:'已复制'}).waitFor();
 await setupPrompt.fill('');assert.ok(await setupDialog.getByRole('button',{name:'复制 Codex 安装指令',exact:true}).isDisabled());await setupPrompt.fill(original);
 for(const mode of ['light','dark'])for(const width of [820,390]){
  await page.evaluate(mode=>window.__setTaskTheme(mode),mode);await page.setViewportSize({width,height:700});
  assert.ok(await setupDialog.evaluate(e=>e.scrollWidth<=e.clientWidth+1),'Setup must not overflow horizontally');
  await page.screenshot({path:resolve(output,`agent-setup-${mode}-${width}.png`)});
 }
 await page.setViewportSize({width:1440,height:920});
 const prompt=setupPrompt;const defaultText=await prompt.inputValue();
 await prompt.fill('只在关键节点合并登记。');await page.getByRole('button',{name:'确认',exact:true}).click();await page.getByRole('status').filter({hasText:'提示词已保存'}).waitFor();await page.keyboard.press('Escape');
 await page.getByLabel('更多任务操作').click();await page.getByRole('button',{name:'接入 Agent',exact:true}).click();assert.equal(await prompt.inputValue(),'只在关键节点合并登记。');
 await page.evaluate(()=>window.__promptConflict());await prompt.fill('冲突保留草稿');await page.getByRole('button',{name:'确认',exact:true}).click();await page.getByRole('alert').filter({hasText:'revision='}).waitFor();assert.equal(await prompt.inputValue(),'冲突保留草稿');await page.keyboard.press('Escape');
 await page.getByLabel('更多任务操作').click();await page.getByRole('button',{name:'接入 Agent',exact:true}).click();await page.getByRole('button',{name:'恢复默认',exact:true}).click();await page.getByRole('status').filter({hasText:'已恢复默认'}).waitFor();assert.equal(await prompt.inputValue(),defaultText);await page.keyboard.press('Escape');

 // Completion and restoration use the panel actions; a new run never changes old evidence.
 await goto('manual-completion');await openRun();
 const stateTrigger=panel.getByRole('button',{name:'执行状态：待确认',exact:true});
 const complete=async()=>{await stateTrigger.click();await panel.getByRole('menuitem',{name:'已完成',exact:true}).click();};
 await stateTrigger.focus();await page.keyboard.press('ArrowDown');await panel.getByRole('menu',{name:'修改执行状态',exact:true}).waitFor();
 assert.ok(await panel.getByRole('menuitem',{name:'已完成',exact:true}).evaluate(e=>e===document.activeElement));
 await page.keyboard.press('Escape');assert.equal(await panel.count(),1,'Escape closes only the state menu');assert.ok(await stateTrigger.evaluate(e=>e===document.activeElement));
 const oldEvidence=await page.evaluate(()=>JSON.stringify(window.__taskCompletionState()[0].runs.at(-1).validationPlan));
 await page.evaluate(()=>window.__holdTaskClose=true);await complete();await page.waitForFunction(()=>document.querySelector('.task-run-panel .run-status-trigger')?.disabled);
 assert.ok(await stateTrigger.isDisabled());await page.evaluate(()=>{window.__holdTaskClose=false;window.__rejectTaskClose();});await panel.getByRole('alert').waitFor();
 assert.equal(await page.evaluate(()=>window.__taskCompletionState()[0].runs.at(-1).status),'ready_for_delivery');
 await page.evaluate(()=>window.__conflictTaskCompletion());await complete();await page.getByRole('alert').filter({hasText:'revision='}).waitFor();
 await close();await page.getByRole('button',{name:'刷新任务',exact:true}).click();await page.getByRole('button',{name:'返回任务列表',exact:true}).click();await page.getByLabel('任务状态',{exact:true}).selectOption('ready_for_delivery');await page.locator('.task-row').first().click();await openRun();
 await complete();await panel.waitFor({state:'detached'});await page.getByRole('status').filter({hasText:'已标记完成'}).waitFor();
 assert.equal(await page.locator('.run-card .task-status').first().innerText(),'已完成');
 assert.ok(await page.locator('.task-heading h3').evaluate(e=>e===document.activeElement));
 const beforeRestore=await page.evaluate(()=>window.__taskCompletionState()[0].runs);assert.equal(JSON.stringify(beforeRestore.at(-1).validationPlan),oldEvidence);assert.equal(beforeRestore.at(-1).closure.accepted,false);
 await openRun();const restore=panel.getByRole('button',{name:'执行状态：已完成',exact:true});const restoreRun=async()=>{await restore.click();await panel.getByRole('menuitem',{name:'开发中',exact:true}).click();};await restoreRun();const dialog=page.getByRole('dialog',{name:'改为开发中',exact:true});await dialog.waitFor();
 assert.equal(await dialog.getByLabel('处理方案',{exact:true}).inputValue(),'');await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});assert.ok(await restore.evaluate(e=>e===document.activeElement));
 await restoreRun();await dialog.waitFor();await dialog.getByLabel('执行标题',{exact:true}).fill('复发后继续处理');await page.evaluate(()=>window.__holdTaskSave=true);await dialog.getByRole('button',{name:'确认',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.form-controls')?.disabled);
 await page.evaluate(()=>{window.__holdTaskSave=false;window.__rejectTaskSave();});await dialog.getByRole('alert').waitFor();assert.equal(await dialog.getByLabel('执行标题',{exact:true}).inputValue(),'复发后继续处理');
 await dialog.getByRole('button',{name:'确认',exact:true}).click();await dialog.waitFor({state:'detached'});await panel.waitFor({state:'detached'});
 const restored=await page.evaluate(()=>window.__taskCompletionState()[0]);assert.deepEqual(restored.runs.slice(0,-1),beforeRestore);assert.equal(restored.runs.at(-1).status,'in_progress');assert.equal(restored.runs.at(-1).validationPlan.records.length,0);
 await openRun(1);assert.equal(await page.getByRole('button',{name:'执行状态：已完成',exact:true}).count(),0,'Closed historical run stays read-only');await close();
 // A completed latest Run must not hide earlier active or pending work.
 await goto('aggregate',false);assert.deepEqual(await page.locator('.column-count').allTextContents(),['1','1','1','1']);
 const activeCard=page.locator('[data-task-id="aggregate-0"]');
 await activeCard.getByLabel('开发中 2 次执行',{exact:true}).waitFor();await activeCard.getByLabel('待确认 1 次执行',{exact:true}).waitFor();
 await page.getByRole('button',{name:'表格',exact:true}).click();
 const activeRow=page.locator('tr').filter({has:page.locator('[data-task-id="aggregate-0"]')});
 assert.equal(await activeRow.locator('.task-status').first().innerText(),'开发中');
 await activeRow.getByLabel('待确认 1 次执行',{exact:true}).waitFor();
 await activeRow.locator('.task-row').click();await page.getByRole('button',{name:'任务状态：开发中',exact:true}).click();
 await page.getByRole('menuitem',{name:'已完成',exact:true}).click();await page.getByRole('status').filter({hasText:'任务已完成并锁定'}).waitFor();
 await page.getByRole('button',{name:'任务状态：已完成',exact:true}).waitFor();
 assert.equal(await page.locator('.run-card').filter({hasText:'开发中'}).count(),2,'Task completion never rewrites Run statuses');
 await page.getByRole('button',{name:'任务状态：已完成',exact:true}).click();await page.getByRole('menuitem',{name:'开发中',exact:true}).click();
 await page.getByRole('dialog',{name:'改为开发中',exact:true}).waitFor();await page.keyboard.press('Escape');
 assert.deepEqual(errors,[]);console.log(`Task UI passed (${engine}): Task board/table and filtering/sorting, overview hierarchy, framework run panel/diff return, responsive themes, keyboard focus, completion/history preservation, prompt editing/conflicts.`);
}catch(e){console.error(e);throw e;}finally{await browser.close();await server.close();}
