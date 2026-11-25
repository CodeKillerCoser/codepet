// Real HTTP MCP process, isolated app data, no frontend or model process.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { request as httpRequest } from 'node:http';
import assert from 'node:assert/strict';
const root=resolve(import.meta.dirname,'..');
const temporary=await mkdtemp(join(tmpdir(),'codepet-task-http-'));
const home=join(temporary,'home'),workspace=join(temporary,'repository');
await mkdir(home);await mkdir(workspace);
const git=args=>execFileSync('git',['-C',workspace,...args],{encoding:'utf8'}).trim();
git(['init','-b','main']);git(['config','user.email','test@example.invalid']);git(['config','user.name','HTTP acceptance']);
await writeFile(join(workspace,'file'),'initial');git(['add','file']);git(['commit','-m','initial']);await writeFile(join(workspace,'file'),'dirty');
const binary=process.env.CODEPET_MCP_BINARY||join(root,'crates/target/debug/codepet-task-mcp');
let sequence=0;
function post(url,headers,payload){return new Promise((resolve,reject)=>{
 const req=httpRequest(url,{method:'POST',agent:false,headers:{accept:'application/json, text/event-stream','content-type':'application/json',...headers}},res=>{
  let body='';res.on('data',d=>body+=d);res.on('end',()=>resolve({status:res.statusCode,headers:res.headers,body}));
 });req.on('error',reject);req.setTimeout(15000,()=>req.destroy(Error('HTTP MCP timeout')));req.end(JSON.stringify(payload));
});}
async function start(){
 const child=spawn(binary,[process.env.CODEPET_MCP_EMBEDDED?'--task-mcp-http':'--http','0'],{env:{...process.env,HOME:home,CODEX_HOME:join(home,'.codex'),USERPROFILE:home,XDG_DATA_HOME:join(home,'data'),LOCALAPPDATA:join(home,'data')},stdio:['ignore','pipe','pipe']});
 let stderr='';child.stderr.on('data',d=>stderr+=d);
 const exited=new Promise((resolveExit,reject)=>{child.on('error',reject);child.on('exit',code=>resolveExit(code));});
 let metadata;
 try{metadata=await new Promise((resolveReady,reject)=>{
  const timer=setTimeout(()=>reject(Error(`HTTP MCP startup timeout: ${stderr}`)),15000);
  createInterface({input:child.stdout}).once('line',line=>{clearTimeout(timer);try{resolveReady(JSON.parse(line));}catch(error){reject(error);}});
  exited.then(code=>{clearTimeout(timer);reject(Error(`HTTP MCP exited early: ${code}: ${stderr}`));},reject);
 });}catch(error){child.kill();throw error;}
 assert.ok(metadata.url.startsWith('http://127.0.0.1:'));
 assert.ok((await realpath(metadata.tokenFile)).startsWith((await realpath(home))+'/'),'Credentials must remain in isolated app data');
 const token=await readFile(metadata.tokenFile,'utf8');let protocol;
 async function rpc(method,params={}){
  const response=await post(metadata.url,{authorization:`Bearer ${token}`,'X-CodePet-Agent':'codex',...(protocol?{'MCP-Protocol-Version':protocol}:{})},{jsonrpc:'2.0',id:++sequence,method,params});
  assert.equal(response.status,200,response.body);assert.equal(response.headers['mcp-session-id'],undefined);
  const body=JSON.parse(response.body);if(body.error)throw Error(JSON.stringify(body.error));return body.result;
 }
 const initialized=await rpc('initialize',{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'http-process-acceptance',version:'1'}});
 protocol=initialized.protocolVersion;assert.equal(initialized.serverInfo.name,'codepet-tasks');
 async function call(name,args){if(['update-task-run','create-task-run'].includes(name))args={agentConversation:['00000000-0000-4000-8000-000000000003'],...args};const result=await rpc('tools/call',{name,arguments:args});if(result.isError)throw Error(result.content.map(c=>c.text).join('\n'));return JSON.parse(result.content[0].text);}
 async function stop(){child.kill('SIGINT');assert.equal(await Promise.race([exited,new Promise((_,reject)=>{const t=setTimeout(()=>{child.kill();reject(Error('HTTP MCP shutdown timeout'));},10000);t.unref();})]),0,stderr);}
 return {child,metadata,token,rpc,call,stop};
}
let first,second;
try{
 first=await start();
 const tools=await first.rpc('tools/list');assert.deepEqual(tools.tools.map(t=>t.name).sort(),['create-task','create-task-run','find-task-runs','finish-task-run','update-task-run']);
 const toolBytes=Buffer.byteLength(JSON.stringify(tools));assert.ok(toolBytes<24000);
 const input={agentWorkspace:{path:workspace},title:'HTTP 验收样例',description:'任务背景',initialRun:{agentConversation:['00000000-0000-4000-8000-000000000003'],title:'首次实现',kind:'bugfix',goal:'修复目标',solution:'修复方案',description:'通过 HTTP 登记并验证',validationPlan:{items:[{title:'可登记并读取',steps:'保存并查询',expected:'任务和执行一致'}]}}};
 const created=await first.call('create-task',input);assert.equal(created.revision,1);assert.equal(created.createdBy,'codex');
 const retry=await first.call('create-task',input);assert.equal(retry.taskId,created.taskId);
 const updated=await first.call('finish-task-run',{taskId:created.taskId,runId:created.runId,expectedRevision:1,agentConversation:['00000000-0000-4000-8000-000000000003'],conclusion:'本机 HTTP 自测完成',changedFiles:['file'],verification:{planRevision:1,stage:'local',target:'HTTP test worktree',environment:process.platform,results:[{criterionId:created.criterionIds[0],result:'passed',evidence:'HTTP 实际读写通过'}]}});assert.equal(updated.revision,2);
 const denied=await post(first.metadata.url,{}, {jsonrpc:'2.0',id:1,method:'tools/list',params:{}});assert.equal(denied.status,401);
 const origin=await post(first.metadata.url,{authorization:`Bearer ${first.token}`,origin:'https://example.invalid'},{jsonrpc:'2.0',id:1,method:'tools/list',params:{}});assert.equal(origin.status,403);
 const missingAgent=await post(first.metadata.url,{authorization:`Bearer ${first.token}`},{jsonrpc:'2.0',id:1,method:'tools/list',params:{}});assert.equal(missingAgent.status,400);
 const token=first.token;await first.stop();first=null;
 second=await start();assert.equal(second.token,token);
 const task=await second.call('find-task-runs',{taskId:created.taskId});assert.equal(task.revision,2);assert.equal(task.taskDescription,'任务背景');assert.equal(task.run.status,'ready_for_delivery');assert.equal(task.run.agentWorkspace.changeStatus.hasUncommittedChanges,true);assert.ok(task.run.agentWorkspace.changedFiles.length);assert.equal(task.run.createdBy,'codex');
 await assert.rejects(second.call('update-task-run',{taskId:created.taskId,runId:created.runId,expectedRevision:1,note:'stale'}),/revision=2/);
 await assert.rejects(second.call('update-task-run',{taskId:created.taskId,runId:created.runId,expectedRevision:2,actor:'human'}));
 await second.stop();second=null;
 const report={success:true,mode:'real-http-without-frontend',toolCount:5,toolDefinitionBytes:toolBytes,createReceiptBytes:Buffer.byteLength(JSON.stringify(created)),updateReceiptBytes:Buffer.byteLength(JSON.stringify(updated)),nativeCli:!!process.env.CODEPET_MCP_EMBEDDED,credentialsPersistedAcrossRestart:true,unauthorizedRejected:true,crossOriginRejected:true,agentCannotImpersonateHuman:true,taskPersistsAcrossRestart:true,revisionConflictPreserved:true,dirtyWorkspaceObserved:true,changedFilesObserved:true,agentSourceFromHeader:true,taskRunIsolation:true};
 await mkdir(join(root,'artifacts/task-ledger-qa'),{recursive:true});await writeFile(join(root,`artifacts/task-ledger-qa/http${process.env.CODEPET_MCP_EMBEDDED?'-embedded':''}-report.json`),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{first?.child.kill();second?.child.kill();await rm(temporary,{recursive:true,force:true});}
