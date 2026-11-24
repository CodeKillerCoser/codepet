// End-to-end production IPC + Rust HTTP using an isolated ledger and prompt configuration.
import {invoke} from '@tauri-apps/api/core';
import {mount, tick} from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';
import {themeClassNames} from '../lib/theme/defaults';
import {taskRequest, currentRun, acceptanceReady, type Task, type AgentSetup} from '../lib/taskLedger';

const pause = () => new Promise(r => setTimeout(r, 40));
async function until<T>(read: () => T | undefined | false): Promise<T> {
  const end = Date.now() + 15000;
  while (Date.now() < end) { const value = read(); if (value) return value; await pause(); }
  throw Error('Task UI state timed out');
}
function check(value: unknown, message: string) { if (!value) throw Error(message); }
async function button(name: string, parent: ParentNode = document) {
  await tick();
  const b = [...parent.querySelectorAll<HTMLButtonElement>('button')].find(b => (b.getAttribute('aria-label')??b.textContent?.trim()) === name);
  if (!b) throw Error(`Missing button: ${name}`);
  b.click(); await tick();
}
function input(label: string, value: string, parent: ParentNode = document) {
  const e = [...parent.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>('input,textarea,select')].find(e => [...(e.labels ?? [])].some(l => l.textContent?.startsWith(label)));
  if (!e) throw Error(`Missing field: ${label}`);
  e.value = value; e.dispatchEvent(new Event(e.tagName === 'SELECT' ? 'change' : 'input', {bubbles: true}));
}
const modal = () => until(() => document.querySelector<HTMLElement>('[role="dialog"]') ?? undefined);
const submit = (dialog: HTMLElement) => dialog.querySelector<HTMLFormElement>('form')!.requestSubmit();
async function dismiss(dialog: HTMLElement) { dialog.querySelector<HTMLButtonElement>('header button')!.click(); await until(() => !dialog.isConnected); }
async function more(name: string) { document.querySelector<HTMLElement>('[aria-label="更多任务操作"]')!.click(); await tick(); await button(name); }

export async function runTaskLedgerNativeQa(expected: {workspace: string; taskId: string}) {
  const target = document.getElementById('app')!;
  target.className = `main-theme ${themeClassNames('light')}`;
  const receipt = await invoke<{revision: number}>('qa_http_task_write');
  let task = await taskRequest<Task>('get', {taskId: expected.taskId});
  check(receipt.revision === task.revision, 'HTTP write reaches App');
  check(task.createdBy === 'codex' && currentRun(task).timeline.at(-1)?.actor === 'native-qa', 'Request-scoped source');
  const background = task.description;
  mount(WorkbenchFrameQa, {target});
  await button('任务与验收'); await until(() => document.querySelector('.run-card'));
  check(!document.querySelector('.task-context')?.textContent?.includes('bugfix'), 'Type belongs to run');
  check(document.querySelector('.run-card')?.textContent?.includes('bugfix'), 'Card contains run type');
  check(![...document.querySelectorAll('button')].some(b => ['更新执行', '结束执行'].includes(b.textContent?.trim() ?? '')), 'Removed run actions');
  check(!acceptanceReady(currentRun(task)), 'Agent self-test is not human acceptance');

  const versions=await taskRequest<{id:string;files:{path:string}[]}[]>('change-diffs',{taskId:task.id,runId:currentRun(task).id});
  check(versions.length>0&&versions[0].files.length>0,'Real commit diff archive is exposed by production IPC');
  const nativeDiff=await taskRequest<{patch:string;error:string|null}>('file-diff',{taskId:task.id,runId:currentRun(task).id,version:versions[0].id,path:versions[0].files[0].path});
  check(!nativeDiff.error&&nativeDiff.patch.includes('@@'),'Actual archived Git diff through production IPC');
  await button('查看变更');await until(()=>document.querySelector('.change-file'));
  document.querySelector<HTMLElement>('.change-file>summary')!.click();await until(()=>document.querySelector('.diff-line'));
  check(document.querySelectorAll('.diff-line.added,.diff-line.deleted').length>0,'Native UI renders Git diff lines');
  await button('关闭代码变更');await until(()=>!document.querySelector('.app-side-panel'));
  check(!document.querySelector('.criterion-history'),'No acceptance history block');

  const recordItem = async (index: number) => {
    const item = document.querySelectorAll<HTMLDetailsElement>('.criterion')[index];
    item.open = true;
    await button('记录结果', item);
    const dialog = await modal();
    input('实际验收版本', '1.2.0 已安装包', dialog);
    input('验收环境', 'macOS', dialog);
    input('验收状态', 'passed', dialog);
    input('验收结论', `实际检查第 ${index + 1} 项，符合预期`, dialog);
    submit(dialog); await until(() => !dialog.isConnected);
    check(document.activeElement === item.querySelector('summary'), 'Result dialog returns to visible row');
  };
  await recordItem(0);
  task = await taskRequest<Task>('get', {taskId: task.id});
  check(!acceptanceReady(currentRun(task)), 'One item does not accept others');
  check(currentRun(task).validationPlan.records.at(-1)?.results.length === 1, 'Single-item result only');
  await recordItem(1);
  task = await taskRequest<Task>('get', {taskId: task.id});
  check(acceptanceReady(currentRun(task)), 'Per-item human results aggregate');
  check(document.querySelectorAll('.check-mark.passed').length === 2, 'Passed items show standalone checks');
  await taskRequest('close-run', {taskId: task.id, runId: currentRun(task).id, expectedRevision: task.revision, accepted: true, reason: '原生人工验收通过'});
  await button('刷新任务'); await pause();
  await more('新一次执行');
  let dialog = await modal();
  input('执行标题', '升级 1.3.0 后菜单再次失效', dialog);
  input('本次目标', '修复新版本的点击问题', dialog);
  input('处理方案', '核对新版事件注册', dialog);
  submit(dialog); await until(() => !dialog.isConnected);
  task = await taskRequest<Task>('get', {taskId: task.id});
  check(task.runs.length === 2 && task.runs[0].closure?.accepted, 'New run retains historical evidence');
  check(!currentRun(task).validationPlan.records.length, 'New run clears results');
  document.querySelectorAll<HTMLButtonElement>('.run-card')[0].click(); await tick();
  check(!document.querySelector('.record-result'), 'History read only');
  document.querySelectorAll<HTMLButtonElement>('.run-card')[1].click(); await tick();
  await taskRequest('update-task-run', {input: {agentConversation: [], taskId: task.id, runId: currentRun(task).id, expectedRevision: task.revision, title: '复发问题已修复', goal: '窗口切换后点击稳定响应', solution: '补齐新版事件注册', kind: 'feature', status: 'ready_for_delivery'}});
  await button('刷新任务'); await pause();
  task = await taskRequest<Task>('get', {taskId: task.id});
  check(task.description === background && currentRun(task).kind === 'feature', 'Run fields independent of Task');
  check(currentRun(task).agentWorkspace.changedFiles.length > 0 && currentRun(task).agentWorkspace.changeStatus.hasUncommittedChanges, 'Actual dirty workspace observed');

  const pending = document.querySelector<HTMLDetailsElement>('.criterion')!;
  pending.open = true; await button('记录结果', pending); dialog = await modal();
  input('实际验收版本', '1.3.0', dialog); input('验收环境', 'macOS', dialog);
  input('验收结论', '过期草稿必须保留', dialog); input('验收状态', 'passed', dialog);
  await taskRequest('update-task-run', {input: {agentConversation: [], taskId: task.id, runId: currentRun(task).id, expectedRevision: task.revision, note: '并发写入'}});
  submit(dialog); await until(() => dialog.querySelector('[role="alert"]')?.textContent?.includes('revision='));
  check(dialog.isConnected && dialog.querySelector('textarea')?.value === '过期草稿必须保留', 'Conflict retains result draft');
  await dismiss(dialog); await button('刷新任务'); await pause();
  task = await taskRequest<Task>('get', {taskId: task.id});
  const solution = currentRun(task).solution;
  await button('修改任务信息'); dialog = await modal();
  input('背景与现象', background + '（人工补充背景）', dialog); submit(dialog); await until(() => !dialog.isConnected);
  task = await taskRequest<Task>('get', {taskId: task.id});
  check(task.description.includes('人工补充背景') && currentRun(task).solution === solution, 'Task edit leaves run solution unchanged');

  await more('接入 Agent'); dialog = await modal();
  check(dialog.querySelector('textarea')?.value.includes('--managed-stdio'), 'Stable stdio config shown');
  const customPrompt = '原生验收：方案确定后登记，完成时合并更新；不要逐消息维护。';
  input('Agent 使用提示词', customPrompt, dialog); await button('保存提示词', dialog);
  await until(() => dialog.querySelector('[role="status"]')?.textContent?.includes('已保存'));
  let initialized = await invoke<{initializeInstructions: string}>('qa_http_task_write');
  check(initialized.initializeInstructions === customPrompt, 'Edited prompt reaches actual HTTP initialize');
  const stdioInit = await invoke<{initializeInstructions: string}>('qa_stdio_initialize');
  check(stdioInit.initializeInstructions === customPrompt, 'Edited prompt reaches stable stdio initialize');
  const setup = await taskRequest<AgentSetup>('setup');
  await taskRequest('save-instructions', {expectedRevision: setup.instructionsRevision, text: '其他窗口已保存的新提示词'});
  input('Agent 使用提示词', '提示词草稿必须保留', dialog); await button('保存提示词', dialog);
  await until(() => dialog.querySelector('[role="alert"]')?.textContent?.includes('revision='));
  check(dialog.querySelector<HTMLTextAreaElement>('[aria-label="Agent 使用提示词"]')?.value === '提示词草稿必须保留', 'Prompt conflict retains draft');
  await dismiss(dialog);
  check(document.activeElement === document.querySelector('[aria-label="更多任务操作"]'), 'Setup restores focus');
  await more('接入 Agent'); dialog = await modal(); await button('恢复默认', dialog);
  await until(() => dialog.querySelector('[role="status"]')?.textContent?.includes('已恢复默认'));
  initialized = await invoke<{initializeInstructions: string}>('qa_http_task_write');
  check(initialized.initializeInstructions === (await taskRequest<AgentSetup>('setup')).defaultInstructions, 'Reset reaches actual HTTP initialize');
  await dismiss(dialog); await button('刷新任务'); await pause();
  await invoke('qa_report', {value: {
    success: true, mode: 'native-task-run-production-ipc', taskId: task.id, workspace: expected.workspace,
    runFields: true, headerSource: true, singleItemAcceptance: true, promptEditing: true,
    promptHttpInitialization: true, promptStdioInitialization: true, promptReset: true, promptConflictPreservesDraft: true,
    humanAcceptance: true, conflictPreservesDraft: true, changedFiles: true,
    runCount: 2, taskSnapshot: await taskRequest<Task>('get', {taskId: task.id})
  }});
}
