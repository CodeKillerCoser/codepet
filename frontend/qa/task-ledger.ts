import {taskLedgerOverflow} from './taskLedgerOverflow';
// Browser layout replay of isolated native evidence; no production writes.
import '../styles.css';import '../main-window.css';
import {mockIPC} from '@tauri-apps/api/mocks';import {mount} from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';import {themeClassNames} from '../lib/theme/defaults';
import TaskRunDetail from '../lib/TaskRunDetail.svelte';
import '../lib/taskLedger.css';
import {currentRun,type Task} from '../lib/taskLedger';
const report=await fetch('/artifacts/task-ledger-qa/native/report.json').then(r=>r.json());
if(!report.success||report.taskSnapshot?.schemaVersion!==4)throw Error('Run native task/run acceptance first');
const base:Task=report.taskSnapshot,tasks=[base],scenario=new URLSearchParams(location.search).get('scenario');
if(scenario==='overflow')tasks.splice(0,1,...taskLedgerOverflow(base));
if(scenario==='overflow'&&new URLSearchParams(location.search).has('manyRuns')){
 const task=tasks[0],run=currentRun(task);
 task.runs=Array.from({length:12},(_,i)=>({...structuredClone(run),id:`overflow-many-${i}`,createdAt:run.createdAt+i*60000}));
}
if(scenario==='single')base.runs=[currentRun(base)];
if(scenario==='long'){base.description='布局样例：任务背景需要保留，但默认只展示摘要。'.repeat(60);currentRun(base).goal='布局样例：这次处理的目标按需展开。'.repeat(60);currentRun(base).solution='布局样例：定位结论与实施方案按需展开。'.repeat(60);}
if(scenario==='dense'){
  const titles=['修复桌宠菜单点击失效','Agent 任务登记与验收清单','历史对话与项目关联','优化工作树差异预览','修复主窗口焦点恢复','完善更新资源包校验','验证新版本安装流程','工作树状态刷新'];
  tasks.splice(0,1,...titles.map((title,i)=>{const task=structuredClone(base);task.id=`layout-${i}`;task.title=title;task.updatedAt-=i*3600000;const run=currentRun(task);run.kind=i%2?'feature':'bugfix';run.status=(['published','in_progress','ready_for_delivery','closed'] as const)[i%4];run.closure=run.status==='closed'?{accepted:false,reason:'布局样例',closedAt:task.updatedAt}:null;return task;}));
  const run=currentRun(tasks[0]),item=run.validationPlan.items[0];run.validationPlan.items=[item,{...item,id:'layout-first-click',title:'首次启动后的菜单响应',steps:'直接点击桌宠打开菜单',expected:'菜单立即出现，点击事件无遗漏'},{...item,id:'layout-long',title:'长时间运行后再次点击',steps:'持续运行后再次点击',expected:'菜单保持响应'}];run.validationPlan.revisions.at(-1)!.criteria=run.validationPlan.items;
  run.validationPlan.records=[{id:'local',actor:'agent',agent:'codex',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'local',target:'布局样例工作区',environment:'macOS',results:run.validationPlan.items.map(c=>({criterionId:c.id,result:'passed',evidence:'布局样例：本地通过'}))},{id:'release',actor:'human',agent:'human',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'release',target:'布局样例 1.3.0',environment:'macOS',results:run.validationPlan.items.slice(0,2).map((c,i)=>({criterionId:c.id,result:i?'failed':'passed',evidence:'布局样例：发布后验证'}))}];
}
// The design scenario is an explicit visual fixture matching the approved mock, not production evidence.
if(scenario==='design'){
 base.title='保留工作台扫描结果';base.description='切换页面或重启 App 后，希望继续查看上一次扫描结果。';
 base.createdAt=new Date('2026-11-22T14:30:00+08:00').getTime();base.createdBy='codex';
 const template=structuredClone(currentRun(base));
 base.runs=['首次实现','修复切页丢失','修复重启丢失'].map((title,i)=>{
  const run=structuredClone(template);run.id=`design-run-${i}`;run.title=title;run.kind=i?'bugfix':'feature';
  run.status=i===2?'ready_for_delivery':'published';run.createdBy='codex';run.createdAt=new Date(['2026-11-22T14:30:00+08:00','2026-11-23T16:20:00+08:00','2026-11-24T19:36:00+08:00'][i]).getTime();
  run.description='';run.goal='重启 App 后仍可查看最近一次扫描结果。';run.solution='持久化项目扫描快照，启动时恢复；重新扫描后更新。';
  run.agentWorkspace={...run.agentWorkspace,path:'/Users/wangxin/Projects/codepet',branch:'codex/restore-scan',changedFiles:[{path:'frontend/lib/session.ts',changeType:'modified',staged:false,originalPath:null},{path:'src-tauri/src/workbench.rs',changeType:'modified',staged:false,originalPath:null},{path:'frontend/lib/session.test.ts',changeType:'modified',staged:false,originalPath:null}],relatedCommits:['abc1234','def5678'],changeStatus:{hasUncommittedChanges:true,hasUnpushedCommits:false}};
  run.agentConversation=[{agent:'codex',conversationId:'00000000-0000-4000-8000-000000000000',verified:true}];
  run.closure=null;
  const item={...template.validationPlan.items[0],scope:'release' as const,precondition:''};
  run.validationPlan.items=['切换页面后保留扫描结果','不同项目的扫描结果互不覆盖','重启 App 后恢复最近扫描结果'].map((title,n)=>({...item,id:`design-check-${n}`,title,steps:'扫描项目 → 退出 App → 重新打开',expected:'项目列表与最近扫描时间保持一致'}));
  run.validationPlan.revisions.at(-1)!.criteria=run.validationPlan.items;
  run.validationPlan.records=[{id:'design-verify',actor:'human',agent:'human',createdAt:run.createdAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'release',target:'布局样例 1.0',environment:'macOS',results:run.validationPlan.items.slice(0,2).map(c=>({criterionId:c.id,result:'passed',evidence:'布局样例：符合预期'}))}];
  run.timeline=[{id:'design-e1',at:run.createdAt,actor:'codex',kind:'created',summary:'创建第 3 次执行'},{id:'design-e2',at:run.createdAt+26*60000,actor:'codex',kind:'milestone',summary:'更新方案与验证计划'},{id:'design-e3',at:run.createdAt+42*60000,actor:'human',kind:'verification',summary:'验收通过 2 项'}];
  return run;
 });
}
if(scenario==='cached-files'){
 const run=currentRun(base);run.agentWorkspace.runFiles=null;run.agentWorkspace.relatedCommits=['cached-a','cached-b'];
 run.agentWorkspace.commitChanges=[{commit:'cached-a',error:null,files:[{path:'cached-added.ts',changeType:'added',staged:false,originalPath:null},{path:'cached-modified.ts',changeType:'modified',staged:false,originalPath:null}]},{commit:'cached-b',error:null,files:[{path:'cached-modified.ts',changeType:'modified',staged:false,originalPath:null},{path:'cached-deleted.ts',changeType:'deleted',staged:false,originalPath:null}]}];
}
if(scenario==='owned-files'){const run=currentRun(base);run.agentWorkspace.runFiles=new URLSearchParams(location.search).has('emptyFiles')?[]:['frontend/feature.svelte'];run.agentWorkspace.changedFiles=[];run.agentWorkspace.relatedCommits=['1234567890abcdef'.repeat(4)];}
if(scenario==='local-verification'){
 const run=currentRun(base);run.validationPlan.items=run.validationPlan.items.map(c=>({...c,scope:'both'}));
 run.validationPlan.records=[{id:'local-only',actor:'agent',agent:'codex',createdAt:base.updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'local',target:'local-commit',environment:'macOS',results:run.validationPlan.items.map(c=>({criterionId:c.id,result:'passed',evidence:'本地检查结论已明确记录'}))}];
}
const defaults='在方案确定后登记任务，实现告一段落时合并更新。保留 taskId/runId/revision，不逐消息或逐文件维护。';
let setupState={config:'[mcp_servers.codepet_tasks]\nurl = "http://127.0.0.1:47623/mcp"\nhttp_headers = { "X-CodePet-Agent" = "codex" }',instructions:defaults,defaultInstructions:defaults,instructionsRevision:0,instructionsCustomized:false};
(window as any).__promptConflict=()=>{setupState.instructionsRevision++;};
mockIPC(async(command,args:any)=>{
 if(command.startsWith('plugin:event|'))return 1;
 if(command==='codex_workbench_request')return args.request.op==='projects'?{projects:[],warnings:[]}:[];
 if(command!=='task_ledger_request')throw Error(`Unexpected ${command}`);
 if(args.request.op==='list')return structuredClone(tasks);
 if(args.request.op==='openChat'){(window as any).__openedTaskChat=args.request.conversationId;return null;}
 if(args.request.op==='commit-files')return (tasks.find(t=>t.id===args.request.taskId)?.runs.find(r=>r.id===args.request.runId)??currentRun(base)).agentWorkspace.relatedCommits.map(commit=>({commit,files:[{path:'committed-change.ts',changeType:'modified',staged:false,originalPath:null}],error:null}));
 if(args.request.op==='change-diffs'){
  const run=tasks.find(t=>t.id===args.request.taskId)?.runs.find(r=>r.id===args.request.runId)??currentRun(base);
  const snapshot=(scenario==='cached-files'?run.agentWorkspace.commitChanges?.flatMap(c=>c.files):undefined)??[{path:'committed-change.ts',changeType:'modified',staged:false,originalPath:null}];
  const fileList=scenario==='owned-files'?(run.agentWorkspace.runFiles??[]).map(path=>({path,changeType:'modified',staged:false,originalPath:null})):[...new Map(snapshot.map(f=>[f.path,f])).values()];
  return [{id:'commit-sample',commit:'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',capturedAt:run.updatedAt,observedAt:run.updatedAt,error:null,files:fileList.map(f=>({...f,added:2,deleted:1,error:null}))}];
 }
 if(args.request.op==='file-diff')return {path:args.request.path,originalPath:null,changeType:'modified',added:2,deleted:1,error:null,patch:'diff --git a/sample.ts b/sample.ts\n--- a/sample.ts\n+++ b/sample.ts\n@@ -1,2 +1,3 @@\n-const old = true;\n+const current = true;\n+const retained = true;\n unchanged\n'};
 if(args.request.op==='setup')return {...setupState};
 if(['update-task-run','edit-task','save-instructions'].includes(args.request.op)&&(window as any).__holdTaskSave)return new Promise((_,reject)=>{(window as any).__rejectTaskSave=()=>reject(Error('模拟保存失败'));});
 if(args.request.op==='save-instructions'){
  if(args.request.expectedRevision!==setupState.instructionsRevision)throw Error('Agent 提示词已更新：revision='+setupState.instructionsRevision);
  setupState={...setupState,instructions:args.request.text??defaults,instructionsRevision:setupState.instructionsRevision+1,instructionsCustomized:args.request.text!==null};return {...setupState};
 }
 throw Error('Layout replay does not simulate domain writes');
});
const target=document.getElementById('app')!;target.className=`main-theme ${themeClassNames('light')}`;
(window as any).__setTaskTheme=(mode:'light'|'dark')=>target.className=`main-theme ${themeClassNames(mode)}`;
if(new URLSearchParams(location.search).has('detailOnly')){
 target.classList.add('task-ledger');target.style.cssText='padding:12px 25px;container-type:inline-size';
 mount(TaskRunDetail,{target,props:{task:base,onEditTask:()=>{},onVerify:()=>{},onOpenChat:()=>{}}});
}else mount(WorkbenchFrameQa,{target,props:{view:'tasks'}});
