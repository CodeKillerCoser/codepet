// Real stdio MCP process, isolated app data, no frontend or model process.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';
const root=resolve(import.meta.dirname,'..');
const temporary=await mkdtemp(join(tmpdir(),'codepet-task-mcp-'));
const workspace=join(temporary,'repo');await mkdir(workspace);
const git=(args)=>execFileSync('git',['-C',workspace,...args],{encoding:'utf8'}).trim();
git(['init','-b','main']);git(['config','user.email','test@example.invalid']);git(['config','user.name','MCP test']);
await writeFile(join(workspace,'file'),'original');git(['add','file']);git(['commit','-m','initial']);
await writeFile(join(workspace,'file'),'dirty');
const home=join(temporary,'home');await mkdir(home);
const codexHome=join(home,'.codex');await mkdir(codexHome);const threadId=randomUUID();
execFileSync('python3',['-c',"import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE threads(id TEXT,cwd TEXT)'); c.execute('INSERT INTO threads VALUES (?,?)',sys.argv[2:]); c.commit()",join(codexHome,'state_5.sqlite'),threadId,workspace]);
const binary=process.env.CODEPET_MCP_BINARY||join(root,'crates/target/debug/codepet-task-mcp');
let count=0;
function client(){
  const child=spawn(binary,process.env.CODEPET_MCP_EMBEDDED?['--task-mcp']:[],{env:{...process.env,HOME:home,CODEX_HOME:codexHome,USERPROFILE:home,XDG_DATA_HOME:join(home,'data'),LOCALAPPDATA:join(home,'data')},stdio:['pipe','pipe','pipe']});
  let stderr='';child.stderr.on('data',d=>stderr+=d);const waiting=new Map();let next=0;
  const lines=createInterface({input:child.stdout});lines.on('line',line=>{let v;try{v=JSON.parse(line);}catch{throw new Error(`Non-protocol stdout: ${line}`);}if(v.id!==undefined){const w=waiting.get(v.id);if(w){clearTimeout(w.timer);waiting.delete(v.id);v.error?w.reject(new Error(JSON.stringify(v.error))):w.resolve(v.result);}}});
  const send=(v)=>child.stdin.write(JSON.stringify({jsonrpc:'2.0',...v})+'\n');
  const request=(method,params={})=>new Promise((resolve,reject)=>{const id=++next;const timer=setTimeout(()=>{waiting.delete(id);reject(new Error(`Timed out: ${method}: ${stderr}`));},20000);waiting.set(id,{resolve,reject,timer});send({id,method,params});});
  const start=async()=>{const init=await request('initialize',{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'codepet-acceptance',version:'1'}});assert.ok(init.capabilities.tools);send({method:'notifications/initialized'});return init;};
  const call=async(name,args)=>{if(['update-task-run','create-task-run'].includes(name))args={agentConversation:[threadId],...args};count++;const result=await request('tools/call',{name,arguments:args});if(result.isError)throw new Error(result.content.map(c=>c.text).join('\n'));return JSON.parse(result.content[0].text);};
  const stop=async()=>{child.stdin.end();await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{child.kill();reject(new Error('MCP failed to exit on stdin EOF'));},10000);child.once('exit',()=>{clearTimeout(timer);resolve();});});};
  return {child,start,request,call,stop};
}
let a,b;
try{
  a=client();await a.start();const tools=await a.request('tools/list');assert.deepEqual(tools.tools.map(t=>t.name).sort(),['create-task','create-task-run','find-tasks','finish-task-run','update-task-run']);const toolBytes=Buffer.byteLength(JSON.stringify(tools));assert.ok(toolBytes<24000);
  const create={agentWorkspace:{path:workspace},title:'修复菜单点击',description:'切换窗口后的首次点击无响应',initialRun:{agentConversation:[threadId],title:'首次实现',kind:'bugfix',goal:'修复目标',solution:'修复方案',description:'排查点击事件绑定',validationPlan:{items:[{title:'切换窗口点击',steps:'切换窗口后点击桌宠',expected:'菜单出现'}]}}};
  const first=await a.call('create-task',create);assert.equal(first.revision,1);assert.equal(first.status,'in_progress');assert.equal(first.createdBy,'codex');assert.ok(Buffer.byteLength(JSON.stringify(first))<1000);
  const repeat=await a.call('create-task',create);assert.equal(repeat.taskId,first.taskId);
  const finish={taskId:first.taskId,runId:first.runId,expectedRevision:first.revision,agentConversation:[threadId],conclusion:'确认事件绑定原因并修复',changedFiles:['file'],agentWorkspace:{path:workspace,relatedCommits:[git(['rev-parse','HEAD'])]},verification:{planRevision:first.planRevision,stage:'local',target:'dirty worktree at HEAD',environment:process.platform,results:[{criterionId:first.criterionIds[0],result:'passed',evidence:'实际本地测试通过'}]}};
  const second=await a.call('finish-task-run',finish);assert.equal(second.revision,2);
  await assert.rejects(a.call('update-task-run',{taskId:first.taskId,runId:first.runId,expectedRevision:2,status:'published'}),/发布版本/);
  const publish={taskId:first.taskId,runId:first.runId,expectedRevision:2,status:'published',delivery:'MCP 验收样例 1.3.0'};
  await a.call('update-task-run',publish);await a.stop();a=null;
  b=client();await b.start();const detail=await b.call('find-tasks',{taskId:first.taskId});assert.equal(detail.revision,3);assert.equal(detail.run.status,'published');assert.equal(detail.description,create.description);assert.equal(detail.run.agentWorkspace.changeStatus.hasUncommittedChanges,true);assert.equal(detail.run.agentWorkspace.relatedCommits.length,1);assert.equal(detail.run.validationPlan.latestVerification.actor,'agent');assert.equal(detail.run.agentConversation[0].verified,true);
  assert.equal((await b.call('update-task-run',publish)).revision,3);
  await assert.rejects(b.call('update-task-run',{taskId:first.taskId,runId:first.runId,expectedRevision:1,note:'stale'}),/revision=3/);
  await assert.rejects(b.call('update-task-run',{taskId:first.taskId,runId:first.runId,expectedRevision:3,status:'closed'}),/人工操作/);
  const next=await b.call('create-task-run',{taskId:first.taskId,expectedRevision:3,title:'发布后复发',kind:'bugfix',goal:'修复复发问题',solution:'重新定位'});assert.notEqual(next.runId,first.runId);
  const active=await b.call('find-tasks',{taskId:first.taskId,history:true});assert.equal(active.runCount,2);assert.equal(active.run.validationPlan.latestVerification,null);assert.notEqual(active.run.validationPlan.items[0].id,first.criterionIds[0]);
  const found=await b.call('find-tasks',{workspace,query:'菜单'});assert.equal(found.total,1);await b.stop();b=null;
  const report={success:true,mode:'real-stdio-without-frontend',toolCount:5,toolDefinitionBytes:toolBytes,receiptBytes:Buffer.byteLength(JSON.stringify(first)),retryAcrossRestart:true,taskRunIsolation:true,historyPreserved:true,changedFilesObserved:true,explicitSourceVerifiedLocally:true,agentCannotClose:true,testCalls:count};
  await mkdir(join(root,'artifacts/task-ledger-qa'),{recursive:true});await writeFile(join(root,'artifacts/task-ledger-qa/mcp-report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{a?.child.kill();b?.child.kill();await rm(temporary,{recursive:true,force:true});}
