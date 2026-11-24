// Replays a snapshot from the real native acceptance run for layout-only browser QA.
import '../styles.css';
import '../main-window.css';
import { mockIPC } from '@tauri-apps/api/mocks';
import { mount } from 'svelte';
import TaskLedger from '../lib/TaskLedger.svelte';
import { themeClassNames } from '../lib/theme/defaults';
import type { Task } from '../lib/taskLedger';
const report=await fetch('/artifacts/task-ledger-qa/native/report.json').then(r=>r.json());
if(!report.success||!report.taskSnapshot)throw new Error('Run native task acceptance first');
const base:Task=report.taskSnapshot;
for(const round of base.rounds)round.phase=['published','validating_release'].includes(round.phase)?'published':['ready_for_delivery','ready_to_publish','awaiting_acceptance','ready_for_release'].includes(round.phase)?'ready_for_delivery':'in_progress';
const tasks=[base];
// Additional layout-only samples exercise list density and different verification states.
if(new URLSearchParams(location.search).get('scenario')==='dense') {
  const titles=['修复桌宠菜单点击失效','Codex 任务登记与验收清单','历史对话与项目关联','优化工作树差异预览','修复主窗口焦点恢复','完善更新资源包校验','验证新版本安装流程','工作树状态刷新'];
  tasks.splice(0,1,...titles.map((title,i)=>{const t=structuredClone(base);t.id=`layout-${i}`;t.title=title;t.updatedAt-=i*3600000;t.rounds.at(-1)!.phase=(['published','in_progress','ready_for_delivery'] as const)[i%3];return t;}));
  for(const [i,task] of tasks.entries())if(i%4===3)task.rounds.at(-1)!.closure={outcome:'closed',reason:'布局样例：本轮已结束',actor:'human',closedAt:task.updatedAt,verificationIds:[]};
  const round=tasks[0].rounds.at(-1)!;
  const plan=round.plans.at(-1)!;
  plan.criteria=[plan.criteria[0],{...plan.criteria[0],id:'layout-first-click',title:'首次启动后的菜单响应',precondition:'首次启动应用',steps:'直接点击桌宠并打开菜单',expected:'菜单立即出现，点击事件无遗漏'},{...plan.criteria[0],id:'layout-long-running',title:'长时间运行后再次点击',precondition:'应用持续运行三十分钟',steps:'反复切换应用后再次点击桌宠',expected:'菜单保持正常响应'}];
  round.verifications=[{id:'layout-local',actor:'agent',createdAt:tasks[0].updatedAt,workspaceObservation:null,planRevision:plan.revision,stage:'local',target:'布局验收样例 · 本地工作区',environment:'macOS 样例',results:plan.criteria.map(c=>({criterionId:c.id,result:'passed',evidence:'布局样例：本地测试通过'}))},{id:'layout-release',actor:'human',createdAt:tasks[0].updatedAt,workspaceObservation:null,planRevision:plan.revision,stage:'release',target:'布局验收样例 · 1.3.0',environment:'macOS 样例',results:plan.criteria.slice(0,2).map((c,i)=>({criterionId:c.id,result:i===0?'passed':'failed',evidence:i===0?'布局样例：菜单正常出现':'布局样例：首次启动时偶发无响应'}))}];
}
mockIPC(async(command,args:any)=>{
  if(command!=='task_ledger_request')throw new Error(`Unexpected ${command}`);
  if(args.request.op==='list')return structuredClone(tasks);
  if(args.request.op==='setup')return {config:'[mcp_servers.codepet]\ncommand = \"fixture\"',instructions:'在方案与验收节点登记任务。'};
  if(args.request.op==='save'&&(window as any).__holdTaskSave)return new Promise((_,reject)=>{(window as any).__rejectTaskSave=()=>reject(new Error('模拟保存失败：验证忙碌时不能关闭窗口'));});
  throw new Error('Layout QA does not simulate business writes');
});
const target=document.getElementById('app')!;
target.className=`main-theme ${themeClassNames('light')}`;
(window as any).__setTaskTheme=(mode:'light'|'dark')=>target.className=`main-theme ${themeClassNames(mode)}`;
const section=document.createElement('section');section.className='workbench';target.appendChild(section);
mount(TaskLedger,{target:section});
