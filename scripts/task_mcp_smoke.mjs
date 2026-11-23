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
  const call=async(name,args)=>{count++;const result=await request('tools/call',{name,arguments:args});if(result.isError)throw new Error(result.content.map(c=>c.text).join('\n'));return JSON.parse(result.content[0].text);};
  const stop=async()=>{child.stdin.end();await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{child.kill();reject(new Error('MCP failed to exit on stdin EOF'));},10000);child.once('exit',()=>{clearTimeout(timer);resolve();});});};
  return {child,start,request,call,stop};
}
let a,b;
try{
  a=client();await a.start();const tools=await a.request('tools/list');assert.deepEqual(tools.tools.map(t=>t.name).sort(),['tasks_find','tasks_save']);const toolBytes=Buffer.byteLength(JSON.stringify(tools));assert.ok(toolBytes<14000);
  const create={workspace,title:'修复菜单点击',objective:'切换窗口后点击也能打开菜单',plan:{summary:'恢复点击事件',criteria:[{title:'切换窗口点击',steps:'切换窗口后点击桌宠',expected:'菜单出现'}]}};
  const first=await a.call('tasks_save',create);assert.equal(first.revision,1);assert.equal(first.status,'in_progress');assert.equal(first.sourceLinked,false);assert.ok(Buffer.byteLength(JSON.stringify(first))<1000);
  const repeat=await a.call('tasks_save',create);assert.equal(repeat.taskId,first.taskId);
  const finish={taskId:first.taskId,expectedRevision:first.revision,phase:'ready_for_delivery',threadId,note:'实现和本地自测结束',commits:[git(['rev-parse','HEAD'])],verification:{planRevision:first.planRevision,stage:'local',target:'dirty worktree at HEAD',environment:process.platform,results:[{criterionId:first.criterionIds[0],result:'passed',evidence:'实际本地测试通过'}]}};
  const second=await a.call('tasks_save',finish);assert.equal(second.revision,2);assert.equal(second.status,'ready_for_delivery');assert.equal(second.sourceLinked,true);
  await assert.rejects(a.call('tasks_save',{taskId:first.taskId,expectedRevision:2,phase:'published'}),/发布版本/);
  const publish={taskId:first.taskId,expectedRevision:2,phase:'published',delivery:'MCP 验收样例 · Release 1.3.0'};
  const third=await a.call('tasks_save',publish);assert.equal(third.revision,3);assert.equal(third.status,'published');
  await a.stop();a=null;
  b=client();await b.start();const detail=await b.call('tasks_find',{taskId:first.taskId});assert.equal(detail.revision,3);assert.equal(detail.status,'published');assert.equal(detail.latestCode.dirty,true);assert.equal(detail.latestCode.commits.length,0);assert.equal(detail.latestVerification.actor,'agent');assert.equal(detail.closure,null);assert.equal(detail.plan.criteria[0].title,'切换窗口点击');
  assert.equal(detail.deliveryFacts.items.find(f=>f.scope==='shared_worktree').label,'待提交');assert.equal(detail.deliveryFacts.items.find(f=>f.scope==='linked_commits').label,'推送待确认');
  const persistedRetry=await b.call('tasks_save',publish);assert.equal(persistedRetry.revision,3);
  await assert.rejects(b.call('tasks_save',{taskId:first.taskId,expectedRevision:1,note:'过期更改'}),/revision=3/);
  await assert.rejects(b.call('tasks_save',{taskId:first.taskId,expectedRevision:3,actor:'human'}));
  const found=await b.call('tasks_find',{workspace,query:'菜单'});assert.equal(found.total,1);assert.equal(found.tasks[0].taskId,first.taskId);
  await b.stop();b=null;
  const report={success:true,mode:'real-stdio-without-frontend',normalMilestoneWrites:3,toolCount:2,toolDefinitionBytes:toolBytes,createReceiptBytes:Buffer.byteLength(JSON.stringify(first)),updateReceiptBytes:Buffer.byteLength(JSON.stringify(second)),retryAcrossRestart:true,dirtyWorkspaceObserved:true,sourceMissingRemainsUnlinked:true,explicitSourceVerifiedLocally:true,agentCannotImpersonateHuman:true,conflictPreservesData:true,publishedRecordedSeparately:true,testCalls:count};
  await mkdir(join(root,'artifacts/task-ledger-qa'),{recursive:true});await writeFile(join(root,'artifacts/task-ledger-qa/mcp-report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{a?.child.kill();b?.child.kill();await rm(temporary,{recursive:true,force:true});}
