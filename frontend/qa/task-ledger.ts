// Browser layout replay of isolated native evidence; no production writes.
import '../styles.css';import '../main-window.css';
import {mockIPC} from '@tauri-apps/api/mocks';import {mount} from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';import {themeClassNames} from '../lib/theme/defaults';
import {currentRun,type Task} from '../lib/taskLedger';
const report=await fetch('/artifacts/task-ledger-qa/native/report.json').then(r=>r.json());
if(!report.success||report.taskSnapshot?.schemaVersion!==3)throw Error('Run native task/run acceptance first');
const base:Task=report.taskSnapshot,tasks=[base],scenario=new URLSearchParams(location.search).get('scenario');
if(scenario==='single')base.runs=[currentRun(base)];
if(scenario==='long'){base.description='布局样例：任务背景需要保留，但默认只展示摘要。'.repeat(60);currentRun(base).description='布局样例：这次处理的定位结论与实施方案按需展开。'.repeat(60);}
if(scenario==='dense'){
  const titles=['修复桌宠菜单点击失效','Agent 任务登记与验收清单','历史对话与项目关联','优化工作树差异预览','修复主窗口焦点恢复','完善更新资源包校验','验证新版本安装流程','工作树状态刷新'];
  tasks.splice(0,1,...titles.map((title,i)=>{const task=structuredClone(base);task.id=`layout-${i}`;task.title=title;task.updatedAt-=i*3600000;task.kind=i%2?'feature':'bugfix';const run=currentRun(task);run.status=(['published','in_progress','ready_for_delivery','closed'] as const)[i%4];run.closure=run.status==='closed'?{accepted:false,reason:'布局样例',closedAt:task.updatedAt}:null;return task;}));
  const run=currentRun(tasks[0]),item=run.validationPlan.items[0];run.validationPlan.items=[item,{...item,id:'layout-first-click',title:'首次启动后的菜单响应',steps:'直接点击桌宠打开菜单',expected:'菜单立即出现，点击事件无遗漏'},{...item,id:'layout-long',title:'长时间运行后再次点击',steps:'持续运行后再次点击',expected:'菜单保持响应'}];run.validationPlan.revisions.at(-1)!.criteria=run.validationPlan.items;
  run.validationPlan.records=[{id:'local',actor:'agent',agent:'codex',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'local',target:'布局样例工作区',environment:'macOS',results:run.validationPlan.items.map(c=>({criterionId:c.id,result:'passed',evidence:'布局样例：本地通过'}))},{id:'release',actor:'human',agent:'human',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'release',target:'布局样例 1.3.0',environment:'macOS',results:run.validationPlan.items.slice(0,2).map((c,i)=>({criterionId:c.id,result:i?'failed':'passed',evidence:'布局样例：发布后验证'}))}];
}
mockIPC(async(command,args:any)=>{
  if(command.startsWith('plugin:event|'))return 1;
  if(command==='codex_workbench_request')return args.request.op==='projects'?{projects:[],warnings:[]}:[];
  if(command!=='task_ledger_request')throw Error(`Unexpected ${command}`);
  if(args.request.op==='list')return structuredClone(tasks);
  if(args.request.op==='setup')return {config:'[mcp_servers.codepet_tasks]\nurl = "http://127.0.0.1:47623/mcp"\nhttp_headers = { "X-CodePet-Agent" = "codex" }',instructions:'布局验收样例：在开始实施与完成时分别登记。'};
  if(['update-task-run','edit-task'].includes(args.request.op)&&(window as any).__holdTaskSave)return new Promise((_,reject)=>{(window as any).__rejectTaskSave=()=>reject(Error('模拟保存失败'));});
  throw Error('Layout replay does not simulate domain writes');
});
const target=document.getElementById('app')!;target.className=`main-theme ${themeClassNames('light')}`;
(window as any).__setTaskTheme=(mode:'light'|'dark')=>target.className=`main-theme ${themeClassNames(mode)}`;
mount(WorkbenchFrameQa,{target,props:{view:'tasks'}});
