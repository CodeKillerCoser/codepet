<script lang="ts">
  import {onMount, onDestroy, tick} from 'svelte';
  import {open} from '@tauri-apps/plugin-dialog';
  import {Ellipsis} from '@lucide/svelte';
  import * as Dialog from './ui/dialog/index';
  import TaskRunDetail from './TaskRunDetail.svelte';
  import TaskAgentSetup from './TaskAgentSetup.svelte';
  import TaskListResizeHandle from './TaskListResizeHandle.svelte';
  import TaskRunChanges from './TaskRunChanges.svelte';
  import AppSidePanel from './shell/AppSidePanel.svelte';
  import {projectRunChanges,type CommitChange} from './taskChanges';
  import {workbenchSession} from './codexWorkbenchSession';
  import {
    taskRequest as api, currentRun, taskStatus, statuses, kinds, scopes, results,
    filterTasks, verificationContext, formatTaskTime as fmt,
    type Task, type TaskRun, type Criterion, type Scope, type TestResult, type TaskKind, type Receipt
  } from './taskLedger';
  import './taskLedger.css';

  export let workspaceHint = '';
  let tasks: Task[] = [], query = '', project = '', status = 'all', selectedId = '', runId = '';
  let layoutWidth=0, preferredListWidth=workbenchSession.page.taskListWidth;
  let changesOpen=false, changesTrigger:HTMLElement|undefined, changesKey='', changesSequence=0;
  let commits:CommitChange[]=[], changesLoading=false, changesError='';
  $: maxListWidth=Math.max(220,Math.min(440,Math.floor(layoutWidth*.45),layoutWidth-368));
  $: listWidth=Math.min(preferredListWidth,maxListWidth);
  let loading = true, busy = false, error = '', message = '', mobileDetail = false;
  let detail: TaskRunDetail, lastButton: HTMLButtonElement | undefined;
  let taskDialog: Dialog.ModalHandle, runDialog: Dialog.ModalHandle, verifyDialog: Dialog.ModalHandle;
  let setupDialog: TaskAgentSetup;
  let taskDraft: Task | null = null, taskTitle = '', taskDescription = '';
  let runTask: Task | null = null, runTitle = '', runKind: TaskKind = 'feature', runGoal = '', runSolution = '', workspace = '';
  let criteria: Criterion[] = [], includePlan = false;
  let verifyTask: Task | null = null, verifyRun: TaskRun | null = null, verifyItem: Criterion | null = null;
  let stage: Scope = 'release', target = '', environment = '', checkResult: TestResult | '' = '', evidence = '';
  let verifyReturnFocus: HTMLElement;
  let moreOpen = false, moreTrigger: HTMLElement;
  let pending: {fingerprint: string; operationId: string} | null = null;
  $: filtered = filterTasks(tasks, query, project, status);
  $: selected = filtered.find(t => t.id === selectedId) ?? filtered[0];
  $: run = selected?.runs.find(r => r.id === runId) ?? (selected && currentRun(selected));
  $: projects = [...new Map(tasks.map(t => [t.projectId, t.projectName])).entries()];
  $: nextChangesKey=selected&&run?[selected.id,run.id,run.agentWorkspace.observedAt,...run.agentWorkspace.relatedCommits].join('|'):'';
  $: if(nextChangesKey!==changesKey){
    if(!changesKey.startsWith(`${selected?.id}|${run?.id}|`))changesOpen=false;
    changesKey=nextChangesKey;const sequence=++changesSequence;
    commits=run?.agentWorkspace.commitChanges??[];changesError='';changesLoading=false;
    if(run&&selected&&!run.agentWorkspace.relatedCommits.every(sha=>commits.some(c=>c.commit===sha&&!c.error))){
      changesLoading=true;void loadCommitFiles(selected.id,run.id,sequence);
    }
  }
  $: changes=run?projectRunChanges(run.agentWorkspace,commits):undefined;
  async function loadCommitFiles(taskId:string,runId:string,sequence:number){
    try {const result=await api<CommitChange[]>('commit-files',{taskId,runId});if(sequence===changesSequence)commits=result;}
    catch(e){if(sequence===changesSequence)changesError=String(e);}
    finally{if(sequence===changesSequence)changesLoading=false;}
  }
  function openChanges(button:HTMLButtonElement){mobileDetail=true;changesTrigger=button;changesOpen=!changesOpen;}
  onDestroy(()=>{++changesSequence;workbenchSession.page.taskListWidth=preferredListWidth;});


  const blank = (): Criterion => ({id: '', title: '', precondition: '', steps: '', expected: '', scope: 'both', required: true});
  async function load() {
    loading = true; error = '';
    try { tasks = await api<Task[]>('list'); }
    catch (e) { error = String(e); }
    finally { loading = false; }
  }
  onMount(() => {
    void load();
    const focus = () => {
      if (!busy && ![taskDialog, runDialog, verifyDialog, setupDialog].some(d => d?.isOpen())) void load();
    };
    window.addEventListener('focus', focus);
    return () => window.removeEventListener('focus', focus);
  });
  async function select(task: Task, button: HTMLButtonElement) {
    selectedId = task.id; runId = ''; lastButton = button; mobileDetail = true;
    await tick(); detail?.focusHeading();
  }
  async function back() { mobileDetail = false; await tick(); lastButton?.focus({preventScroll: true}); }
  async function directory() {
    const path = await open({directory: true, multiple: false});
    if (typeof path === 'string') workspace = path;
  }
  function editTask(task: Task | null) {
    taskDraft = task ? structuredClone(task) : null; runTask = null;
    taskTitle = task?.title ?? ''; taskDescription = task?.description ?? '';
    workspace = workspaceHint; runTitle = ''; runGoal = ''; runSolution = ''; runKind = 'feature';
    includePlan = false; criteria = [blank()]; error = '';
    taskDialog.showModal();
  }
  function createRun() {
    moreOpen = false;
    if (!selected) return;
    runTask = structuredClone(selected);
    const current = currentRun(runTask);
    runTitle = ''; runGoal = current.goal; runSolution = ''; runKind = 'bugfix';
    workspace = current.agentWorkspace.path;
    criteria = structuredClone(current.validationPlan.items).map(c => ({...c, id: '', steps: [c.precondition,c.steps].filter(Boolean).join('\n'),precondition:''}));
    if (!criteria.length) criteria = [blank()];
    includePlan = current.validationPlan.items.length > 0;
    error = ''; runDialog.showModal();
  }
  async function write(op: string, input: Record<string, unknown>) {
    const fingerprint = JSON.stringify({op, input});
    if (pending?.fingerprint !== fingerprint) pending = {fingerprint, operationId: crypto.randomUUID()};
    const receipt = await api<Receipt>(op, {input: {...input, ...(op === 'edit-task' ? {} : {operationId: pending.operationId})}});
    pending = null; selectedId = receipt.taskId;
    await load();
    return receipt;
  }
  async function saveTask() {
    busy = true; error = '';
    try {
      if (taskDraft) {
        await write('edit-task', {taskId: taskDraft.id, expectedRevision: taskDraft.revision, title: taskTitle, description: taskDescription});
      } else {
        await write('create-task', {
          title: taskTitle, description: taskDescription, agentWorkspace: {path: workspace},
          initialRun: {agentConversation: [], title: runTitle || taskTitle, kind: runKind, goal: runGoal || taskTitle, solution: runSolution,
            ...(includePlan ? {validationPlan: {items: criteria}} : {})}
        });
        status = 'all'; runId = '';
      }
      taskDialog.close(); message = '任务已保存';
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function saveRun() {
    if (!runTask) return;
    busy = true; error = '';
    try {
      const receipt = await write('create-task-run', {
        agentConversation: [], taskId: runTask.id, expectedRevision: runTask.revision,
        title: runTitle, kind: runKind, goal: runGoal, solution: runSolution,
        agentWorkspace: {path: workspace}, ...(includePlan ? {validationPlan: {items: criteria}} : {})
      });
      runId = receipt.runId; runDialog.close(); message = '已创建新的执行，历史记录已保留';
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  function setVerificationContext() {
    if (verifyRun) ({target, environment} = verificationContext(verifyRun, stage));
  }
  function startVerification(item: Criterion, returnFocus: HTMLElement) {
    if (!selected || !run) return;
    verifyTask = structuredClone(selected); verifyRun = structuredClone(run); verifyItem = structuredClone(item);
    stage = item.scope === 'local' ? 'local' : 'release';
    setVerificationContext(); checkResult = ''; evidence = ''; error = '';
    verifyReturnFocus = returnFocus;
    verifyDialog.showModal();
  }
  async function saveVerification() {
    if (!verifyTask || !verifyRun || !verifyItem) return;
    if (!checkResult || !evidence.trim()) { error = '请选择验收状态并填写结论'; return; }
    busy = true; error = '';
    try {
      await write('update-task-run', {
        agentConversation: [], taskId: verifyTask.id, runId: verifyRun.id, expectedRevision: verifyTask.revision,
        verification: {planRevision: verifyRun.validationPlan.revision, stage, target, environment,
          results: [{criterionId: verifyItem.id, result: checkResult, evidence}]},
        note: `${scopes[stage]}验收「${verifyItem.title}」：${results[checkResult]} · ${target}`
      });
      verifyDialog.close(); message = '此项验收结果已记录';
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function showSetup() {
    moreOpen = false; moreTrigger?.focus(); error = '';
    try { await setupDialog.show(); } catch (e) { error = String(e); }
  }
  async function openChat(conversationId: string) {
    try { await api('openChat', {taskId: selected?.id, conversationId}); }
    catch (e) { error = String(e); }
  }
</script>

<svelte:window on:click={e => { if (moreOpen && !(e.target instanceof Element && e.target.closest('.ledger-more'))) moreOpen = false; }}
  on:keydown={e => { if (e.key === 'Escape' && moreOpen) { e.preventDefault(); moreOpen = false; moreTrigger?.focus(); } }}/>

{#snippet checklistEditor()}
  <label class="checkbox"><input type="checkbox" bind:checked={includePlan}/>{runTask ? '编辑验收清单（未编辑时沿用上一轮）' : '登记验收清单'}</label>
  {#if includePlan}
    {#each criteria as criterion, i}
      <fieldset class="check-editor"><legend>验收项 {i + 1}</legend>
        <label>验收点<input required bind:value={criterion.title}/></label>
        <label>验收方式<textarea required rows="2" bind:value={criterion.steps}></textarea></label>
        <label>预期结果<textarea required rows="2" bind:value={criterion.expected}></textarea></label>
        <div class="form-row"><label>适用阶段<select bind:value={criterion.scope}>{#each Object.entries(scopes) as [value, label]}<option {value}>{label}</option>{/each}</select></label>
          <label class="checkbox"><input type="checkbox" bind:checked={criterion.required}/>必验</label>
          <button type="button" disabled={criteria.length === 1} on:click={() => criteria = criteria.filter((_, n) => n !== i)}>移除</button>
        </div>
      </fieldset>
    {/each}
    <button type="button" disabled={criteria.length >= 30} on:click={() => criteria = [...criteria, blank()]}>添加验收项</button>
  {/if}
{/snippet}
{#snippet runEditor()}
  <label>执行标题<input maxlength="160" bind:value={runTitle} placeholder="简短说明这次处理什么"/></label>
  <label>执行类型<select bind:value={runKind}>{#each Object.entries(kinds) as [value, label]}<option {value}>{label}</option>{/each}</select></label>
  <label>本次目标<textarea rows="2" maxlength="8000" bind:value={runGoal}></textarea></label>
  <label>处理方案<textarea rows="3" maxlength="8000" bind:value={runSolution}></textarea></label>
  {@render checklistEditor()}
{/snippet}

<section class="task-ledger" aria-label="任务与验收">
  <div class="toolbar">
    <div class="filters" role="search" aria-label="筛选任务">
      <input aria-label="搜索任务" placeholder="搜索任务、背景或处理结论" bind:value={query}/>
      <select aria-label="任务项目" bind:value={project}><option value="">全部项目</option>{#each projects as [id, name]}<option value={id}>{name}</option>{/each}</select>
      <select aria-label="任务状态" bind:value={status}><option value="all">全部状态</option>{#each Object.entries(statuses) as [value, label]}<option {value}>{label}</option>{/each}</select>
    </div>
    <div class="toolbar-actions">
      <button class="quiet" disabled={busy || loading} on:click={load}>刷新任务</button>
      <button class="primary" disabled={busy} on:click={() => editTask(null)}>新建任务</button>
      <details class="ledger-more" open={moreOpen}>
        <summary aria-label="更多任务操作" aria-expanded={moreOpen} title="更多任务操作" bind:this={moreTrigger} on:click|preventDefault={() => moreOpen = !moreOpen}><Ellipsis size={16}/></summary>
        <div class="more-menu"><button class="quiet" on:click={showSetup}>接入 Agent</button>
          {#if selected && ['published', 'closed'].includes(currentRun(selected).status)}<button class="quiet" on:click={createRun}>新一次执行</button>{/if}
        </div>
      </details>
    </div>
  </div>
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  {#if message}<p class="message" role="status">{message}</p>{/if}
  <div class="ledger-layout" class:detail-open={mobileDetail} bind:clientWidth={layoutWidth} style:--task-list-width={`${listWidth}px`}>
    <section class="task-list" id="task-ledger-list" aria-label="任务列表"><small class="result-count">{filtered.length} 个结果</small>
      <div class="task-rows">
        {#each filtered as task (task.id)}
          <button class="task-row" class:selected={selected?.id === task.id} aria-pressed={selected?.id === task.id} on:click={e => select(task, e.currentTarget)}>
            <span class="row-top"><strong title={task.title}>{task.title}</strong><span class="status-badge" data-status={currentRun(task).status}>{taskStatus(task)}</span></span>
            <span class="row-context"><span>{task.projectName} · {task.runs.length} 次执行</span><small>{fmt(task.updatedAt)}</small></span>
          </button>
        {:else}
          <div class="empty">{loading ? '读取任务…' : tasks.length ? '没有符合条件的任务' : '还没有登记任务'}{#if !loading && !tasks.length}<p>连接 Agent 在关键节点登记，也可以手动新建。</p>{/if}</div>
        {/each}
      </div>
    </section>
    <TaskListResizeHandle value={listWidth} max={maxListWidth} active={layoutWidth>700} onResize={value=>preferredListWidth=value}/>
    <aside class="task-detail" aria-label="任务详情" tabindex="-1">
      {#if selected && run}
        <button class="back quiet" on:click={back}>← 返回任务列表</button>
        <TaskRunDetail task={selected} bind:runId bind:this={detail} onEditTask={() => editTask(selected)} onVerify={startVerification} onOpenChat={openChat} {changes} {changesLoading} {changesOpen} onOpenChanges={openChanges}/>
      {:else}<div class="empty">选择任务查看执行记录</div>{/if}
    </aside>
  </div>
</section>

<AppSidePanel open={changesOpen&&!!run} title="代码变更" description={selected?.title??'所选执行的代码变更'} closeLabel="关闭代码变更" returnFocus={changesTrigger} onClose={()=>changesOpen=false}>
  {#if run&&changes}<TaskRunChanges taskId={selected!.id} {run} {changes} {commits} loading={changesLoading} error={changesError}/>{/if}
</AppSidePanel>

<Dialog.Modal title={taskDraft ? '修改任务信息' : '新建任务'} class="ledger-dialog" bind:this={taskDialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={saveTask}><fieldset class="form-controls" disabled={busy}>
    <header><h3>{taskDraft ? '修改任务信息' : '新建任务'}</h3><button type="button" aria-label="关闭任务编辑" on:click={() => taskDialog.close()}>×</button></header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <label>任务标题<input required maxlength="160" bind:value={taskTitle} placeholder="一句话描述要解决什么"/></label>
    <label>背景与现象<textarea rows="3" maxlength="8000" bind:value={taskDescription}></textarea></label>
    {#if !taskDraft}
      <label>工作区<div class="directory"><input required bind:value={workspace}/><button type="button" on:click={directory}>选择</button></div></label>
      <details><summary>首次执行与验收（可选）</summary>{@render runEditor()}</details>
    {/if}
    <footer><button class="primary">{busy ? '保存中…' : '保存任务'}</button></footer>
  </fieldset></form>
</Dialog.Modal>

<Dialog.Modal title="新一次执行" class="ledger-dialog" returnFocus={moreTrigger} bind:this={runDialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={saveRun}><fieldset class="form-controls" disabled={busy}>
    <header><h3>新一次执行</h3><button type="button" aria-label="关闭执行编辑" on:click={() => runDialog.close()}>×</button></header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <label>工作区<div class="directory"><input required bind:value={workspace}/><button type="button" on:click={directory}>选择</button></div></label>
    {@render runEditor()}
    <footer><small>新执行不继承旧验收结果。</small><button class="primary" disabled={!runTitle.trim()}>{busy ? '保存中…' : '创建执行'}</button></footer>
  </fieldset></form>
</Dialog.Modal>

<Dialog.Modal title="记录验收结果" class="ledger-dialog" returnFocus={verifyReturnFocus} bind:this={verifyDialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={saveVerification}><fieldset class="form-controls" disabled={busy}>
    <header><h3>记录验收结果</h3><button type="button" aria-label="关闭验收结果" on:click={() => verifyDialog.close()}>×</button></header>
    <p>{verifyItem?.title}</p>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <label>验收阶段<select bind:value={stage} on:change={setVerificationContext}>
      {#if verifyItem?.scope !== 'release'}<option value="local">本地</option>{/if}
      {#if verifyItem?.scope !== 'local'}<option value="release">发布后</option>{/if}
    </select></label>
    <label>实际验收版本<input required bind:value={target}/></label>
    <label>验收环境<input required bind:value={environment}/></label>
    <label>验收状态<select aria-label="验收状态" required bind:value={checkResult}><option value="" disabled>选择验收状态</option>{#each Object.entries(results) as [value, label]}<option {value}>{label}</option>{/each}</select></label>
    <label>验收结论<textarea required rows="3" maxlength="3000" bind:value={evidence}></textarea></label>
    <footer><button class="primary">{busy ? '保存中…' : '保存此项结果'}</button></footer>
  </fieldset></form>
</Dialog.Modal>

<TaskAgentSetup bind:this={setupDialog} returnFocus={moreTrigger}/>
