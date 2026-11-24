// Real production Rust IPC and HTTP; all task samples live in an isolated QA ledger.
import {invoke} from '@tauri-apps/api/core';
import {mount,tick} from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';
import {themeClassNames} from '../lib/theme/defaults';
import {taskRequest,currentRun,acceptanceReady,type Task} from '../lib/taskLedger';
const pause=()=>new Promise(r=>setTimeout(r,40));
async function until<T>(read:()=>T|undefined|false):Promise<T>{const end=Date.now()+15000;while(Date.now()<end){const value=read();if(value)return value;await pause();}throw Error('Task UI state timed out');}
function check(value:unknown,message:string){if(!value)throw Error(message);}
async function button(name:string,parent:ParentNode=document){const b=[...parent.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent?.trim()===name);if(!b)throw Error(`Missing button: ${name}`);b.click();await tick();}
function input(label:string,value:string,parent:ParentNode=document){const element=[...parent.querySelectorAll<HTMLInputElement|HTMLTextAreaElement|HTMLSelectElement>('input,textarea,select')].find(e=>[...(e.labels??[])].some(l=>l.textContent?.startsWith(label)));if(!element)throw Error(`Missing field: ${label}`);element.value=value;element.dispatchEvent(new Event(element.tagName==='SELECT'?'change':'input',{bubbles:true}));}
const modal=()=>until(()=>document.querySelector<HTMLElement>('[role="dialog"]')??undefined);
const submit=(dialog:HTMLElement)=>dialog.querySelector<HTMLFormElement>('form')!.requestSubmit();
async function dismiss(dialog:HTMLElement){dialog.querySelector<HTMLButtonElement>('header button')!.click();await until(()=>!dialog.isConnected);}
export async function runTaskLedgerNativeQa(expected:{workspace:string;taskId:string}){
  document.getElementById('app')!.className=`main-theme ${themeClassNames('light')}`;
  const receipt=await invoke<{taskId:string;revision:number}>('qa_http_task_write');
  let task=await taskRequest<Task>('get',{taskId:expected.taskId});
  check(receipt.revision===task.revision,'HTTP writes reach the App ledger');
  check(task.createdBy==='codex'&&currentRun(task).timeline.at(-1)?.actor==='native-qa','Header author is request scoped and does not overwrite creator');
  const originalDescription=task.description;
  mount(WorkbenchFrameQa,{target:document.getElementById('app')!});await button('任务与验收');await until(()=>document.querySelector('.task-row'));
  check(document.querySelector('.kind-tag')?.textContent==='修复','bugfix is a task marker');
  check(document.querySelector('.run-heading .status-badge')?.textContent==='已发布','Published status is retained');
  check(!document.querySelector('[aria-label="任务执行历史"]'),'Single run has no selector');
  await button('结束执行');let dialog=await modal();check(dialog.querySelector<HTMLInputElement>('input[type="checkbox"]')?.disabled,'Agent self-test cannot accept');await dismiss(dialog);
  await button('记录人工验收');dialog=await modal();input('实际验收版本','1.2.0 已安装包',dialog);input('验收环境','macOS',dialog);input('结果','passed',dialog);input('证据或说明','实际切换窗口后点击，菜单正常打开',dialog);submit(dialog);await until(()=>!dialog.isConnected);
  task=await taskRequest<Task>('get',{taskId:task.id});check(acceptanceReady(currentRun(task)),'Human evidence satisfies acceptance');
  await button('结束执行');dialog=await modal();input('结束说明','1.2.0 人工验收通过',dialog);submit(dialog);await until(()=>!dialog.isConnected);
  await button('新一次执行');dialog=await modal();input('本次处理原因','升级 1.3.0 后菜单再次失效',dialog);submit(dialog);await until(()=>!dialog.isConnected);
  task=await taskRequest<Task>('get',{taskId:task.id});check(task.runs.length===2&&task.runs[0].closure?.accepted,'New run preserves the first accepted run');check(!currentRun(task).validationPlan.records.length&&!acceptanceReady(currentRun(task)),'Checklist reuse never reuses results');
  const history=document.querySelector<HTMLSelectElement>('[aria-label="任务执行历史"]')!;history.value=task.runs[0].id;history.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>document.querySelector('.closure')?.textContent?.includes('1.2.0'));check(![...document.querySelectorAll('button')].some(b=>b.textContent?.trim()==='更新执行'),'History is read only');history.value='';history.dispatchEvent(new Event('change',{bubbles:true}));await pause();
  await button('更新执行');dialog=await modal();input('处理摘要','已确认新一轮原因并修复，等待交付',dialog);input('执行状态','ready_for_delivery',dialog);submit(dialog);await until(()=>!dialog.isConnected);
  task=await taskRequest<Task>('get',{taskId:task.id});check(task.description===originalDescription,'Run update does not rewrite task background');check(currentRun(task).agentWorkspace.changeStatus.hasUncommittedChanges===true,'Rust sees dirty shared workspace');check(currentRun(task).agentWorkspace.changedFiles.length>0,'Rust exposes real changed files');
  await button('更新执行');dialog=await modal();await taskRequest('update-task-run',{input:{taskId:task.id,runId:currentRun(task).id,expectedRevision:task.revision,note:'并发写入'}});input('处理摘要','过期草稿必须保留',dialog);submit(dialog);await until(()=>dialog.querySelector('[role="alert"]')?.textContent?.includes('revision='));check(dialog.isConnected,'Conflict retains draft');await dismiss(dialog);await button('刷新任务');await pause();
  task=await taskRequest<Task>('get',{taskId:task.id});const executionDescription=currentRun(task).description;
  await button('修改任务信息');dialog=await modal();input('背景与现象',originalDescription+'（人工补充背景）',dialog);submit(dialog);await until(()=>!dialog.isConnected);
  task=await taskRequest<Task>('get',{taskId:task.id});check(task.description.includes('人工补充背景')&&currentRun(task).description===executionDescription,'Task metadata edit leaves run description unchanged');
  document.getElementById('app')!.className=`main-theme ${themeClassNames('dark')}`;
  document.querySelector<HTMLElement>('[aria-label="更多任务操作"]')!.click();await tick();await button('接入 Agent');dialog=await modal();const config=dialog.querySelector('textarea')?.value??'';check(config.includes('X-CodePet-Agent')&&config.includes('url = "http://127.0.0.1:'),'Setup contains HTTP source header');await dismiss(dialog);check(document.activeElement===document.querySelector('[aria-label="更多任务操作"]'),'Setup restores visible menu trigger');
  await invoke('qa_report',{value:{success:true,mode:'native-task-run-production-ipc',taskId:task.id,workspace:expected.workspace,roundCount:2,runCount:2,headerSource:true,taskRunIsolation:true,taskMetadataIsolation:true,changedFiles:true,humanAcceptance:true,conflictPreservesDraft:true,httpAgentWriteSeenByUi:true,setupRestoresMenuFocus:true,taskSnapshot:await taskRequest<Task>('get',{taskId:task.id})}});
}
