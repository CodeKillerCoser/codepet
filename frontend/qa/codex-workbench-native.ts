// Native-only QA entry. No mock IPC or fixture snapshots.
import '../styles.css';
import '../main-window.css';
import { invoke } from '@tauri-apps/api/core';
import { mount } from 'svelte';
import { themeClassNames } from '../lib/theme/defaults';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';
import type { Snapshot } from '../lib/codexWorkbench';
window.addEventListener('error',event=>{void invoke('qa_report',{value:{success:false,error:event.message}});});
window.addEventListener('unhandledrejection',event=>{void invoke('qa_report',{value:{success:false,error:String(event.reason)}});});
const pause=()=>new Promise(resolve=>setTimeout(resolve,50));
async function until<T>(read:()=>T|undefined|false):Promise<T> {
  const end=Date.now()+75000;
  while(Date.now()<end){const result=read();if(result)return result;await pause();}
  throw new Error('Native UI acceptance timed out');
}
async function run() {
  const expected=await invoke<{workspace:string;threadId:string;projectId:string;gitEntries:number;mode?:string;taskId?:string}>('qa_config');
  if(expected.mode==='tasks') { const {runTaskLedgerNativeQa}=await import('./task-ledger-native');await runTaskLedgerNativeQa(expected as {workspace:string;taskId:string});return; }
  const actual=await invoke<Snapshot>('codex_workbench_request',{request:{op:'scan',projectIds:[expected.projectId]}});
  document.getElementById('app')!.className=`main-theme ${themeClassNames('light')}`;
  mount(WorkbenchFrameQa,{target:document.getElementById('app')!});
  const scan=await until(()=>{const e=document.querySelector<HTMLButtonElement>('.confirm');return e&&!e.disabled?e:undefined;});
  scan.click();
  await until(()=>document.querySelector('.worktree-row'));
  const target=actual.worktrees.find(row=>row.path===expected.workspace);
  if(!target)throw new Error('Current workspace absent from the native scan');
  if(target.git.error)throw new Error(target.git.error);
  if(target.git.changes.length!==expected.gitEntries||!target.git.changes.length)throw new Error(`Git entry mismatch: ${target.git.changes.length} vs ${expected.gitEntries}`);
  if(!target.conversations.some(c=>c.id===expected.threadId))throw new Error('Current conversation is not associated with its worktree');
  const row=await until(()=>Array.from(document.querySelectorAll<HTMLElement>('.worktree-row')).find(row=>row.querySelector('.identity small')?.getAttribute('title')===expected.workspace));
  row.querySelector<HTMLButtonElement>('.row-select')!.click();
  await until(()=>document.querySelector('.detail .path')?.textContent===expected.workspace);
  const gitLabel=row.querySelector('.git-state')?.textContent??'';
  if(!gitLabel.includes('未提交'))throw new Error('Current workspace is not shown as dirty in the UI');
  const summary=document.querySelector('.git-summary')?.textContent;
  const rechecked=await invoke<any>('codex_workbench_request',{request:{op:'recheck',scanId:actual.scanId+1,worktreeId:target.id}});
  if(rechecked.git.changes.length!==expected.gitEntries)throw new Error('Native recheck diverged from Git baseline');
  const change=target.git.changes.find(c=>c.path==='frontend/lib/CodexWorkbench.svelte')??target.git.changes[0];
  const fileButton=await until(()=>Array.from(document.querySelectorAll<HTMLButtonElement>('.change button')).find(b=>b.getAttribute('aria-label')===`预览 ${change.path}`));
  fileButton.click();await until(()=>document.querySelector('.cm-mergeView'));
  const previewWorked=!!document.querySelector('.cm-editor');
  document.querySelector<HTMLButtonElement>('[aria-label="关闭文件预览"]')!.click();
  await pause();
  await invoke('qa_report',{value:{success:true,mode:'native-production-component-and-ipc',workspace:target.path,threadId:expected.threadId,projectId:target.projectId,projectName:target.projectName,branch:target.git.branch,gitEntryCount:target.git.changes.length,gitBaselineCount:expected.gitEntries,dirtyLabel:gitLabel,detailSummary:summary,currentThreadAssociated:true,recheckWorked:true,relatedConversationCount:target.conversations.length,previewWorked,previewFile:change.path,codexHome:actual.sourceInfo?.codexHome,totalWorktrees:actual.worktrees.length,scanId:actual.scanId,changes:target.git.changes.map(c=>({path:c.path,status:c.status}))}});
}
run().catch(async error=>{await invoke('qa_report',{value:{success:false,error:String(error),stack:error?.stack}});});
