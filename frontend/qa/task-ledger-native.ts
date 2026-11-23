// Native UI exercises production Rust commands and the production Svelte component, no mocks.
import { invoke } from '@tauri-apps/api/core';
import { mount, tick } from 'svelte';
import CodexWorkbench from '../lib/CodexWorkbench.svelte';
import { themeClassNames } from '../lib/theme/defaults';
import { taskRequest, currentRound, acceptanceReady, type Task } from '../lib/taskLedger';

const delay=()=>new Promise(r=>setTimeout(r,40));
async function until<T>(read:()=>T|undefined|false):Promise<T>{const end=Date.now()+15000;while(Date.now()<end){const v=read();if(v)return v;await delay();}throw new Error('Task UI state timed out');}
const check=(condition:unknown,message:string)=>{if(!condition)throw new Error(message);};
async function button(name:string,parent:ParentNode=document){const b=[...parent.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent?.trim()===name);if(!b)throw new Error(`Missing button: ${name}`);b.click();await tick();}
function input(label:string,value:string,parent:ParentNode=document){const el=[...parent.querySelectorAll<HTMLInputElement|HTMLTextAreaElement|HTMLSelectElement>('input,textarea,select')].find(e=>[...(e.labels??[])].some(l=>l.textContent?.startsWith(label)));if(!el)throw new Error(`Missing field: ${label}`);el.value=value;el.dispatchEvent(new Event(el.tagName==='SELECT'?'change':'input',{bubbles:true}));return el;}
function submit(dialog:HTMLDialogElement){dialog.querySelector<HTMLFormElement>('form')!.requestSubmit();}

export async function runTaskLedgerNativeQa(expected:{workspace:string;taskId:string}) {
  document.getElementById('app')!.className=`main-theme ${themeClassNames('light')}`;
  mount(CodexWorkbench,{target:document.getElementById('app')!});await button('任务与验收');
  await until(()=>document.querySelector<HTMLButtonElement>('.task-row'));
  const filter=document.querySelector<HTMLSelectElement>('[aria-label="任务状态"]')!;
  check([...filter.options].map(o=>o.textContent).join(',')==='全部状态,进行中,待交付,已发布,已结束','Only the four main task statuses are exposed');
  check(document.querySelector('.phase-label')?.textContent==='已发布','Published remains a separate status before acceptance');
  check(document.querySelector('.task-detail')?.textContent?.includes('Codex 自测'),'Agent result is labelled as self-test');
  await button('结束任务');let dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;
  check(dialog.querySelector<HTMLInputElement>('input[type="checkbox"]')?.disabled,'Self-test cannot unlock human acceptance');
  dialog.close();await button('记录人工验收');dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;
  input('实际验收版本','1.2.0 已安装包',dialog);input('验收环境','macOS 真实安装环境',dialog);input('结果','passed',dialog);input('证据或说明','实际切换窗口后反复点击，菜单正常打开',dialog);submit(dialog);
  await until(()=>!dialog.open);let task=await taskRequest<Task>('get',{taskId:expected.taskId});check(acceptanceReady(currentRound(task)),'Human result satisfies acceptance');check(currentRound(task).verifications.length===2,'Agent and human runs coexist');
  await button('结束任务');dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;input('结束说明','版本 1.2.0 已安装并验证通过',dialog);submit(dialog);await until(()=>!dialog.open);await until(()=>document.querySelector('.task-detail')?.textContent?.includes('版本 1.2.0 已安装并验证通过'));
  await button('重新开启');dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;input('复发或重新处理的原因','升级 1.3.0 后菜单再次失效',dialog);submit(dialog);await until(()=>!dialog.open);
  task=await taskRequest<Task>('get',{taskId:expected.taskId});check(task.rounds.length===2,'Reopening appends a new round');check(task.rounds[0].closure?.outcome==='accepted','Old acceptance remains');check(task.rounds[1].phase==='in_progress','Reopened task starts in progress');check(task.rounds[1].verifications.length===0,'New round does not inherit passes');check(task.rounds[1].plans.length===1,'Checklist reused without model calls');check(!acceptanceReady(task.rounds[1]),'New round needs new verification');
  const selector=document.querySelector<HTMLSelectElement>('[aria-label="处理轮次"]')!;selector.value=task.rounds[0].id;selector.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>document.querySelector('.closure')?.textContent?.includes('1.2.0'));check(![...document.querySelectorAll('button')].some(b=>b.textContent?.trim()==='记录人工验收'),'Historical round is read-only');
  selector.value='';selector.dispatchEvent(new Event('change',{bubbles:true}));await delay();await button('编辑与记录进展');dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;
  input('任务状态','ready_for_delivery',dialog);input('进展摘要','本轮实现完成，等待交付',dialog);submit(dialog);await until(()=>!dialog.open);
  task=await taskRequest<Task>('get',{taskId:task.id});check(currentRound(task).phase==='ready_for_delivery','Ready for delivery is editable');
  check(currentRound(task).deliveryFacts?.summary.includes('待提交（共享工作树）'),'Shared dirty workspace is scoped explicitly in Rust delivery facts');
  await until(()=>document.querySelector('.delivery-hint')?.textContent?.includes('待提交（共享工作树）'));await button('编辑与记录进展');dialog=document.querySelector<HTMLDialogElement>('dialog[open]')!;
  // A concurrent Agent write must not silently overwrite or be overwritten by the open form.
  await taskRequest('save',{input:{taskId:task.id,expectedRevision:task.revision,note:'并发更新'}});input('进展摘要','使用过期表单保存',dialog);submit(dialog);await until(()=>dialog.querySelector('[role="alert"]')?.textContent?.includes('revision='));check(dialog.open,'Conflicting form retains user draft');dialog.close();
  await taskRequest('get',{taskId:task.id});await button('刷新任务');await delay();
  document.getElementById('app')!.className=`main-theme ${themeClassNames('dark')}`;
  await button('接入 Codex');dialog=await until(()=>document.querySelector<HTMLDialogElement>('dialog[open]')??undefined);check(dialog.textContent?.includes('Rust'),'Setup explains native service');check(dialog.querySelector('textarea')?.value.includes('--task-mcp'),'Config starts native CLI');dialog.close();
  await invoke('qa_report',{value:{success:true,mode:'native-task-ledger-production-ipc',taskId:task.id,workspace:expected.workspace,roundCount:2,agentSelfTestSeparate:true,humanAcceptance:true,reopenPreservesHistory:true,reusedChecklistStartsUntested:true,conflictPreservesDraft:true,frontendIsNotMcpTransport:true,taskSnapshot:await taskRequest<Task>('get',{taskId:task.id})}});
}
