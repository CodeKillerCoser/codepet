import {taskLedgerOverflow} from './taskLedgerOverflow';
// Deterministic browser layout fixture; no production writes or native evidence claims.
import '../styles.css';import '../main-window.css';
import {mockIPC} from '@tauri-apps/api/mocks';import {mount} from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';import {themeClassNames} from '../lib/theme/defaults';
import TaskRunDetail from '../lib/TaskRunDetail.svelte';
import '../lib/taskLedger.css';
import {currentRun,type Task,type RunStatus} from '../lib/taskLedger';
import {taskLedgerFixture} from './taskLedgerFixture';
import defaults from '../../crates/codepet-task-mcp/instructions.md?raw';
const base:Task=taskLedgerFixture(),tasks=[base],scenario=new URLSearchParams(location.search).get('scenario');
(base as any).completion=null;
if(scenario==='overflow')tasks.splice(0,1,...taskLedgerOverflow(base));
if(scenario==='overflow'&&new URLSearchParams(location.search).has('manyRuns')){
 const task=tasks[0],run=currentRun(task);
 task.runs=Array.from({length:12},(_,i)=>({...structuredClone(run),id:`overflow-many-${i}`,createdAt:run.createdAt+i*60000}));
}
if(scenario==='single')base.runs=[currentRun(base)];
if(scenario==='long'){base.description='布局样例：任务背景需要保留，但默认只展示摘要。'.repeat(60);currentRun(base).goal='布局样例：这次处理的目标按需展开。'.repeat(60);currentRun(base).solution='布局样例：定位结论与实施方案按需展开。'.repeat(60);}
// Short text may still occupy many lines in a narrow native sidebar.
if(scenario==='wrapped'){
 base.description='任务背景需要完整阅读。\n第二行保留问题来源。\n第三行说明复发条件。\n第四行包含背景末尾证据。';
 const run=currentRun(base);
 run.goal='检查真实侧栏内的阅读体验，确保较短的中文目标也能在空间不足时展开。'.repeat(3)+'目标末尾证据。';
 run.solution='方案第一行。\n方案第二行。\n方案第三行。\n方案末尾证据。';
 run.timeline=[{id:'wrapped-event',at:run.updatedAt,actor:'codex',kind:'milestone',summary:'活动第一行。\n活动第二行。\n活动第三行。\n活动末尾证据。'}];
}
if(scenario==='dense'){
  const titles=['修复桌宠菜单点击失效','Agent 任务登记与验收清单','历史对话与项目关联','优化工作树差异预览','修复主窗口焦点恢复','完善更新资源包校验','验证新版本安装流程','工作树状态刷新'];
  tasks.splice(0,1,...titles.map((title,i)=>{const task=structuredClone(base);task.id=`layout-${i}`;task.title=title;task.updatedAt-=i*3600000;const run=currentRun(task);run.kind=i%2?'feature':'bugfix';run.status=(['published','in_progress','ready_for_delivery','closed'] as const)[i%4];run.closure=run.status==='closed'?{accepted:false,reason:'布局样例',closedAt:task.updatedAt}:null;return task;}));
  const run=currentRun(tasks[0]),item=run.validationPlan.items[0];run.validationPlan.items=[item,{...item,id:'layout-first-click',title:'首次启动后的菜单响应',steps:'直接点击桌宠打开菜单',expected:'菜单立即出现，点击事件无遗漏'},{...item,id:'layout-long',title:'长时间运行后再次点击',steps:'持续运行后再次点击',expected:'菜单保持响应'}];run.validationPlan.revisions.at(-1)!.criteria=run.validationPlan.items;
  run.validationPlan.records=[{id:'local',actor:'agent',agent:'codex',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'local',target:'布局样例工作区',environment:'macOS',results:run.validationPlan.items.map(c=>({criterionId:c.id,result:'passed',evidence:'布局样例：本地通过'}))},{id:'release',actor:'human',agent:'human',createdAt:tasks[0].updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'release',target:'布局样例 1.3.0',environment:'macOS',results:run.validationPlan.items.slice(0,2).map((c,i)=>({criterionId:c.id,result:i?'failed':'passed',evidence:'布局样例：发布后验证'}))}];
}
if(scenario==='activity'){
 base.title='跨 Agent 协作与活动记录';
 const run=currentRun(base),at=run.updatedAt;
 run.timeline=[{id:'legacy-created',at:at-120000,actor:'unknown',kind:'created',summary:'历史创建记录原样保留'},
 {id:'claude-comment',at:at-90000,actor:'claude',kind:'comment',summary:'已经在另一个工作区确认协议行为，建议补充并发评论的回归验证。',agentConversation:[{agent:'claude',conversationId:'claude-chat-alpha',verified:false}]},
 {id:'cursor-plan',at:at-60000,actor:'cursor',kind:'plan_updated',summary:'修订验收计划：第 1 版 → 第 2 版',changes:[{field:'validationPlan',before:{revision:1,items:run.validationPlan.items},after:{revision:2,items:[{...run.validationPlan.items[0],title:'多个 Agent 的评论不能互相覆盖'}]}}]},
 {id:'state-event',at:at-30000,actor:'codex',kind:'status_changed',summary:'状态：开发中 → 待确认',changes:[{field:'status',before:'in_progress',after:'ready_for_delivery'}]},
 {id:'codex-reply',at,actor:'codex',kind:'comment',summary:'并发追加和重试已验证，之前的活动没有修改。',replyTo:'claude-comment',agentConversation:[{agent:'codex',conversationId:'00000000-0000-4000-8000-000000000001',verified:false}]}];
 (window as any).__appendAgentComment=()=>{run.timeline.push({id:'incoming-comment',at:Date.now(),actor:'cursor',kind:'comment',summary:'来自其他 Agent 的新评论已同步显示',replyTo:'codex-reply'});base.revision++;};
}
// The design scenario is an explicit visual fixture matching the approved mock, not production evidence.
if(scenario==='design'||scenario==='reading'){
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
if(scenario==='reading'){
 const run=currentRun(base);
 run.title='核对验收与时间线的字体层级、长文本换行和展开收起';
 run.validationPlan.items[0].title='长标题验收点：同一项在多行展示时，勾选图标仍与标题第一行对齐。'.repeat(3);
 run.validationPlan.items[0].steps='打开全局执行侧栏，展开验收项。\n检查标签与正文对齐，并分别在窄屏和桌面分栏下阅读。';
 run.validationPlan.items[0].expected='同组字段紧凑排列，验收与活动之间留有清楚间隔，长路径和结论完整换行。';
 run.validationPlan.records[0].results[0].evidence='布局样例：长验收结论包含实际观察、环境和证据，保留全文而不改变任何验收状态。'.repeat(5);
 run.timeline.at(-1)!.summary='布局样例：记录实际检查的结论与处理进展，长事件默认显示摘要，展开后完整查看，不覆盖历史。'.repeat(12)+'末尾证据仍可读取。';
 run.timeline.at(-1)!.reference='/Users/qa/project/artifacts/'+'evidence-long-path-'.repeat(12)+'.png';
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
if(scenario==='manual-completion'){
 const run=currentRun(base);run.status='ready_for_delivery';run.closure=null;
 run.validationPlan.records=[{id:'previous-verification',actor:'agent',agent:'codex',createdAt:base.updatedAt,agentWorkspace:null,planRevision:run.validationPlan.revision,stage:'local',target:'old-build',environment:'test',results:run.validationPlan.items.map(c=>({criterionId:c.id,result:'passed',evidence:'旧验收结论'}))}];
 for(const historical of base.runs.slice(0,-1))historical.status='closed';
 const developing=structuredClone(base);developing.id='completion-developing';developing.title='开发中的任务';currentRun(developing).id='developing-run';currentRun(developing).status='in_progress';
 tasks.push(developing);
}
if(scenario==='aggregate'){
 tasks.splice(0,1,...([
  ['开发中汇总',['in_progress','in_progress','ready_for_delivery','closed']],
  ['待确认汇总',['ready_for_delivery','closed']],
  ['已发布汇总',['published','closed']],
  ['全部完成汇总',['closed','closed']]
 ] as [string,RunStatus[]][]).map(([title,states],i)=>{
  const task=structuredClone(base),template=structuredClone(currentRun(task));task.id=`aggregate-${i}`;task.title=title;
  task.runs=states.map((status,n)=>({...structuredClone(template),id:`aggregate-${i}-${n}`,status}));return task;
 }));
}
// Isolated IPC stand-in only. Production aggregation is exclusively Rust Task::summary.
function qaView(task:Task):Task {
 const counts={total:task.runs.length,inProgress:0,readyForDelivery:0,published:0,closed:0};
 const keys={in_progress:'inProgress',ready_for_delivery:'readyForDelivery',published:'published',closed:'closed'} as const;
 for(const run of task.runs)counts[keys[run.status]]++;
 const manuallyCompleted=Boolean((task as any).completion);
 const state:RunStatus=manuallyCompleted?'closed':counts.inProgress?'in_progress':counts.readyForDelivery?'ready_for_delivery':counts.published?'published':'closed';
 return {...structuredClone(task),summary:{status:state,runCounts:counts,manuallyCompleted}};
}
(window as any).__taskCompletionState=()=>structuredClone(tasks);
(window as any).__conflictTaskCompletion=()=>{base.revision++;};
const agentClients=['codex','claude','cursor','opencode','antigravity'].map((id,i)=>({id,name:['Codex','Claude Code','Cursor','OpenCode','Antigravity'][i],configPath:['~/.codex/config.toml','.mcp.json','~/.cursor/mcp.json','~/.config/opencode/opencode.jsonc','~/.gemini/config/mcp_config.json'][i],rulePath:['~/.codex/AGENTS.md','~/.claude/CLAUDE.md','Cursor → Customize → Rules → User Rules','~/.config/opencode/AGENTS.md','~/.gemini/GEMINI.md'][i],config:id==='codex'?'[mcp_servers.codepet_tasks]\ncommand = \"/Users/qa/.codepet/bin/codepet-task-mcp\"\nargs = [\"--managed-stdio\"]\nenv = { CODEPET_AGENT = \"codex\" }':JSON.stringify(id==='opencode'?{mcp:{servers:{codepet_tasks:{type:'local',command:['/Users/qa/.codepet/bin/codepet-task-mcp','--managed-stdio'],environment:{CODEPET_AGENT:id}}}}}:{mcpServers:{codepet_tasks:{...(id==='claude'?{type:'stdio'}:{}),command:'/Users/qa/.codepet/bin/codepet-task-mcp',args:['--managed-stdio'],env:{CODEPET_AGENT:id}}}},null,2)}));
let setupState={clients:agentClients,transport:'stdio',config:'[mcp_servers.codepet_tasks]\ncommand = "/Users/qa/.codepet/bin/codepet-task-mcp"\nargs = ["--managed-stdio"]\nenv = { CODEPET_AGENT = "codex" }',instructions:defaults,defaultInstructions:defaults,instructionsRevision:0,instructionsCustomized:false};
(window as any).__promptConflict=()=>{setupState.instructionsRevision++;};
mockIPC(async(command,args:any)=>{
 if(command.startsWith('plugin:event|'))return 1;
 if(command==='codex_workbench_request')return args.request.op==='projects'?{projects:[],warnings:[]}:[];
 if(command!=='task_ledger_request')throw Error(`Unexpected ${command}`);
 if(args.request.op==='recall-history'&&args.request.action==='enqueue'){
  if(new URLSearchParams(location.search).has('processingError'))throw Error('请先在设置 → 模型配置中启用任务召回并配置模型');
  const task=tasks.find(task=>task.id===args.request.taskId);
  (window as any).__processingRequests=[...((window as any).__processingRequests??[]),args.request];
  return {taskId:task?.id,queued:task?.runs.filter(run=>run.status!=='in_progress').length??0,existing:0};
 }
 if(args.request.op==='list')return tasks.map(qaView);
 if(args.request.op==='get')return qaView(tasks.find(t=>t.id===args.request.taskId)!);
 if(args.request.op==='complete-task'){
  const task=tasks.find(t=>t.id===args.request.taskId)!;
  if(args.request.expectedRevision!==task.revision)throw Error('任务已更新：revision='+task.revision);
  (task as any).completion={accepted:false,reason:args.request.reason,closedAt:Date.now()};task.revision++;
  return {taskId:task.id,revision:task.revision};
 }
 if(args.request.op==='close-run'){
  if((window as any).__holdTaskClose)return new Promise((_,reject)=>{(window as any).__rejectTaskClose=()=>reject(Error('模拟完成保存失败'));});
  const task=tasks.find(t=>t.id===args.request.taskId);if(!task)throw Error('任务不存在');
  if(args.request.expectedRevision!==task.revision)throw Error('任务已更新：revision='+task.revision+'；请刷新后重试');
  const run=currentRun(task);if(run.id!==args.request.runId||run.status!=='ready_for_delivery')throw Error('只能完成当前待确认执行');
  if(args.request.accepted!==false)throw Error('完成不能伪造人工验收');
  run.status='closed';run.closure={accepted:false,reason:args.request.reason,closedAt:Date.now()};task.revision++;
  run.timeline.push({id:'manual-completion-event',at:Date.now(),actor:'human',kind:'closed',summary:args.request.reason});
  return {taskId:task.id,runId:run.id,revision:task.revision,planRevision:run.validationPlan.revision,criterionIds:run.validationPlan.items.map(c=>c.id),status:run.status,createdBy:run.createdBy};
 }
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
 if(['update-task-run','edit-task','create-task-run','save-instructions'].includes(args.request.op)&&(window as any).__holdTaskSave)return new Promise((_,reject)=>{(window as any).__rejectTaskSave=()=>reject(Error('模拟保存失败'));});
 if(args.request.op==='create-task-run'){
  const input=args.request.input,task=tasks.find(t=>t.id===input.taskId);if(!task)throw Error('任务不存在');
  if(task.revision!==input.expectedRevision)throw Error('任务已更新：revision='+task.revision);
  const previous=currentRun(task);if(!(task as any).completion&&!['closed','published'].includes(previous.status))throw Error('当前执行仍在处理中');
  const items=structuredClone(input.validationPlan?.items??previous.validationPlan.items).map(c=>({...c,id:crypto.randomUUID()}));
  const at=Date.now(),run={...structuredClone(previous),id:crypto.randomUUID(),title:input.title,kind:input.kind,goal:input.goal,solution:input.solution,description:'',status:'in_progress' as const,closure:null,createdAt:at,updatedAt:at,createdBy:'human',agentConversation:[],timeline:[{id:crypto.randomUUID(),at,actor:'human',kind:'created',summary:'用户手动改为开发中：'+input.title}],validationPlan:{revision:1,items,records:[],revisions:[{revision:1,createdAt:at,summary:'',affectedAreas:[],criteria:items}]},agentWorkspace:{...structuredClone(previous.agentWorkspace),path:input.agentWorkspace.path,runFiles:null,relatedCommits:[],commitChanges:[]}};
  (task as any).completion=null;task.runs.push(run);task.revision++;task.updatedAt=at;
  return {taskId:task.id,runId:run.id,revision:task.revision,planRevision:1,criterionIds:items.map(c=>c.id),status:run.status,createdBy:'human'};
 }
 if(args.request.op==='save-instructions'){
  if(args.request.expectedRevision!==setupState.instructionsRevision)throw Error('Agent 提示词已更新：revision='+setupState.instructionsRevision);
  setupState={...setupState,instructions:args.request.text??defaults,instructionsRevision:setupState.instructionsRevision+1,instructionsCustomized:args.request.text!==null};return {...setupState};
 }
 throw Error('Layout replay does not simulate domain writes');
});
const target=document.getElementById('app')!;
const taskThemeParams=new URLSearchParams(location.search);
target.className=`main-theme ${themeClassNames(taskThemeParams.get('theme')==='dark'?'dark':'light',taskThemeParams.get('preset') as any)}`;
(window as any).__setTaskTheme=(mode:'light'|'dark',preset?:any)=>target.className=`main-theme ${themeClassNames(mode,preset)}`;
if(new URLSearchParams(location.search).has('detailOnly')){
 target.classList.add('task-ledger');target.style.cssText='padding:12px 25px;container-type:inline-size';
 mount(TaskRunDetail,{target,props:{task:qaView(base),onEditTask:()=>{},onOpenRun:()=>{}}});
}else mount(WorkbenchFrameQa,{target,props:{view:'tasks'}});
