<script lang="ts">
  import {onMount, onDestroy, tick} from 'svelte';
  import {open} from '@tauri-apps/plugin-dialog';
  import {Ellipsis, Columns3, Table2, ArrowLeft, RefreshCw, X, Trash2, ListFilter} from '@lucide/svelte';
  import TaskStatus from './TaskStatus.svelte';
  import * as Dialog from './ui/dialog/index';
  import TaskRunDetail from './TaskRunDetail.svelte';
  import TaskRunContent from './TaskRunContent.svelte';
  import TaskAgentSetup from './TaskAgentSetup.svelte';
  import TaskCollection from './TaskCollection.svelte';
  import TaskRunChanges from './TaskRunChanges.svelte';
  import AppSidePanel from './shell/AppSidePanel.svelte';
  import {projectRunChanges,type CommitChange} from './taskChanges';
  import {workbenchSession} from './codexWorkbenchSession';
  import {
    taskRequest as api, currentRun, statuses, kinds, scopes,
    filterTasks,
    type Task, type Criterion, type TaskKind, type Receipt, type RunStatus
  } from './taskLedger';
  import './taskLedger.css';

  export let workspaceHint = '';
  const preferences=workbenchSession.page.taskLedger;
  let tasks:Task[]=[],query=preferences.query,project=preferences.project,status=preferences.status,selectedId='',runId='';
  let view=preferences.view,sort=preferences.sort;
  let availableWidth=0;
  $: boardFits=availableWidth>=960;
  $: if(availableWidth>0&&boardFits!==preferences.boardFits){
    preferences.boardFits=boardFits;
    view=boardFits?'board':'table';
  }
  let collection:TaskCollection,lastTaskId='';
  let runPanelOpen=false, panelView:'run'|'changes'='run', runTrigger:HTMLElement|undefined;
  let runContent:TaskRunContent;
  let panelPages:HTMLDivElement, runScrollPosition=0;
  let panelTaskId='';
  let changesKey='', changesSequence=0;
  let commits:CommitChange[]=[], changesLoading=false, changesError='';
  let loading = true, busy = false, error = '', message = '', processingError = '';
  let detail: TaskRunDetail;
  let taskDialog: Dialog.ModalHandle, runDialog: Dialog.ModalHandle;
  let runReturnFocus: HTMLElement | undefined;
  let setupDialog: TaskAgentSetup;
  let taskDraft: Task | null = null, taskTitle = '', taskDescription = '';
  let runTask: Task | null = null, runTitle = '', runKind: TaskKind = 'feature', runGoal = '', runSolution = '', workspace = '';
  let criteria: Criterion[] = [], includePlan = false;
  let moreOpen = false, moreTrigger: HTMLElement;
  let pending: {fingerprint: string; operationId: string} | null = null;
  $: filtered = filterTasks(tasks, query, project, status);
  $: selected = tasks.find(t => t.id === selectedId);
  $: run = selected?.runs.find(r => r.id === runId) ?? (selected && currentRun(selected));
  $: restoringDevelopment = runTask ? runTask.summary.status === 'closed' : false;
  $: projects = [...new Map(tasks.map(t => [t.projectId, t.projectName])).entries()];
  $: if(panelTaskId!==(selected?.id??'')){panelTaskId=selected?.id??'';runPanelOpen=false;panelView='run';}
  $: nextChangesKey=selected&&run?[selected.id,run.id,run.agentWorkspace.observedAt,...run.agentWorkspace.relatedCommits].join('|'):'';
  $: if(nextChangesKey!==changesKey){
    if(!changesKey.startsWith(`${selected?.id}|${run?.id}|`))panelView='run';
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
  function openRun(id:string,button:HTMLButtonElement){runId=id;runTrigger=button;panelView='run';runPanelOpen=true;}
  async function openChanges(button:HTMLButtonElement){runScrollPosition=button.closest('.panel-body')?.scrollTop??0;panelView='changes';await tick();panelPages?.closest('.panel-body')?.scrollTo({top:0});}
  async function backToRun(){panelView='run';await tick();panelPages?.closest('.panel-body')?.scrollTo({top:runScrollPosition});runContent?.focusChanges();}
  onDestroy(()=>{++changesSequence;Object.assign(preferences,{view,sort,query,project,status});});


  const blank = (): Criterion => ({id: '', title: '', precondition: '', steps: '', expected: '', scope: 'both', required: true});
  async function load() {
    loading = true; error = '';
    try { const loaded=await api<Task[]>('list');if(loaded.some(task=>!task.summary))throw Error('任务状态汇总需要更新原生 App（API 33）');tasks=loaded; }
    catch (e) { error = String(e); }
    finally { loading = false; }
  }
  onMount(() => {
    void load();
    let active=true,reading=false;
    const refreshActivity=async()=>{
      if(reading||document.hidden||!selectedId||!runPanelOpen||busy||[taskDialog,runDialog,setupDialog].some(dialog=>dialog?.isOpen()))return;
      const id=selectedId;reading=true;
      try{
        const latest=await api<Task>('get',{taskId:id});
        if(active&&selectedId===id&&!busy&&![taskDialog,runDialog,setupDialog].some(dialog=>dialog?.isOpen())&&latest.revision>(tasks.find(task=>task.id===id)?.revision??0))tasks=tasks.map(task=>task.id===id?latest:task);
      }catch{/* Keep the visible snapshot. Explicit refresh reports service errors. */}
      finally{reading=false;}
    };
    const activityTimer=setInterval(()=>void refreshActivity(),5000);
    const focus = () => {
      if (!busy && ![taskDialog, runDialog, setupDialog].some(d => d?.isOpen())) void load();
    };
    window.addEventListener('focus', focus);
    return () => {active=false;clearInterval(activityTimer);window.removeEventListener('focus', focus);};
  });
  async function select(task: Task, _button: HTMLButtonElement) {
    selectedId=task.id;runId='';lastTaskId=task.id;runPanelOpen=false;
    await tick(); detail?.focusHeading();
  }
  async function back(){lastTaskId=selectedId;selectedId='';runId='';runPanelOpen=false;await tick();collection?.focusTask(lastTaskId);}
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
  function createRun(returnFocus?: HTMLElement) {
    moreOpen = false;
    if (busy || !selected || (selected.summary.status!=='closed'&&!['closed','published'].includes(currentRun(selected).status))) return;
    runTask = structuredClone(selected);
    const current = currentRun(runTask);
    runReturnFocus = returnFocus ?? moreTrigger;
    runTitle = selected.summary.status === 'closed' ? '继续处理' : ''; runGoal = current.goal; runSolution = ''; runKind = 'bugfix';
    workspace = current.agentWorkspace.path;
    criteria = structuredClone(current.validationPlan.items).map(c => ({...c, id: '', steps: [c.precondition,c.steps].filter(Boolean).join('\n'),precondition:''}));
    if (!criteria.length) criteria = [blank()];
    includePlan = selected.summary.status !== 'closed' && current.validationPlan.items.length > 0;
    error = ''; runDialog.showModal();
  }
  async function write(op: string, input: Record<string, unknown>) {
    const fingerprint = JSON.stringify({op, input});
    if (pending?.fingerprint !== fingerprint) pending = {fingerprint, operationId: crypto.randomUUID()};
    const receipt = await api<Receipt>(op, {input: {...input, ...(op === 'edit-task' ? {} : {operationId: pending.operationId})}});
    pending = null; selectedId = receipt.taskId;
    if (op === 'create-task-run' && ['closed','published'].includes(status)) status = 'all';
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
      runId = receipt.runId; runPanelOpen=false;runDialog.close(); message = '已新增一次执行，状态为开发中；之前的验收和历史已保留';
      await tick();detail?.focusHeading();
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function completeRun() {
    if (busy || !selected || selected.summary.manuallyCompleted || !run || run.id !== currentRun(selected).id || run.status !== 'ready_for_delivery') return;
    const taskId=selected.id, completedRunId=run.id, expectedRevision=selected.revision;
    busy=true;error='';message='';
    try {
      await api<Receipt>('close-run',{taskId,runId:completedRunId,expectedRevision,accepted:false,reason:'用户标记完成'});
      selectedId=taskId;runId=completedRunId;
      // Keep the completed task visible when the current filter is pending confirmation.
      if (status==='ready_for_delivery') status='all';
      await load();
      runPanelOpen=false;message='已标记完成';
      await tick();detail?.focusHeading();
    } catch(e) {error=String(e);}
    finally {busy=false;}
  }
  async function completeTask() {
    if(busy||!selected||selected.summary.status==='closed')return;
    const taskId=selected.id,expectedRevision=selected.revision;
    busy=true;error='';message='';
    try{
      await api<Receipt>('complete-task',{taskId,expectedRevision,reason:'用户完成整个任务'});
      if(status!=='all')status='all';
      await load();runPanelOpen=false;message='任务已完成并锁定';
      await tick();detail?.focusHeading();
    }catch(e){error=String(e);}finally{busy=false;}
  }
  async function showSetup() {
    moreOpen = false; moreTrigger?.focus(); error = '';
    try { await setupDialog.show(); } catch (e) { error = String(e); }
  }
  function processingFeedback(value:string,failed:boolean){processingError=failed?value:'';message=failed?'':value;}
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
          <button type="button" class="quiet" aria-label={`删除验收项 ${i+1}`} title="删除" disabled={criteria.length === 1} on:click={() => criteria = criteria.filter((_, n) => n !== i)}><Trash2 size={16}/></button>
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

<section class="task-ledger" aria-label="任务与验收" bind:clientWidth={availableWidth}>
  <div class="toolbar" class:task-toolbar={!!selected}>
    {#if selected}
      <button class="quiet collection-back" disabled={busy} on:click={back} aria-label="返回任务列表"><ArrowLeft size={16}/>全部任务</button>
      <span class="breadcrumb-title" title={selected.title}>{selected.title}</span>
    {:else}
      <div class="collection-heading"><h3>全部任务</h3><span>{filtered.length}</span></div>
      <div class="view-switch" role="group" aria-label="任务视图">
        <button class="ui-selectable" class:active={view==='board'} class:selected={view==='board'} aria-pressed={view==='board'} disabled={!boardFits} title={boardFits?'看板':'内容区宽度不足，已自动使用表格'} on:click={()=>view='board'}><Columns3 size={15}/>看板</button>
        <button class="ui-selectable" class:active={view==='table'} class:selected={view==='table'} aria-pressed={view==='table'} on:click={()=>view='table'}><Table2 size={15}/>表格</button>
      </div>
      <div class="filters" role="search" aria-label="筛选任务">
        <input aria-label="搜索任务" placeholder="搜索任务…" bind:value={query}/>
        <select aria-label="任务项目" bind:value={project}><option value="">全部项目</option>{#each projects as [id,name]}<option value={id}>{name}</option>{/each}</select>
        <div class="status-filter"><span>{#if status==='all'}<ListFilter size={14}/>{:else}<TaskStatus status={status as RunStatus} label={false}/>{/if}</span><select aria-label="任务状态" bind:value={status}><option value="all">全部状态</option>{#each Object.entries(statuses) as [value,label]}<option {value}>{label}</option>{/each}</select></div>
      </div>
    {/if}
    <div class="toolbar-actions">
      <button class="quiet refresh-tasks" aria-label="刷新任务" title="刷新任务" disabled={busy||loading} on:click={load}><RefreshCw size={16}/></button>
      <button class="primary" disabled={busy} on:click={()=>editTask(null)}>新建任务</button>
      <details class="ledger-more" open={moreOpen}>
        <summary aria-label="更多任务操作" aria-expanded={moreOpen} title="更多任务操作" bind:this={moreTrigger} on:click|preventDefault={()=>moreOpen=!moreOpen}><Ellipsis size={16}/></summary>
        <div class="more-menu"><button class="quiet" on:click={showSetup}>接入 Agent</button>
        </div>
      </details>
    </div>
  </div>
  {#if error && !runPanelOpen}<p role="alert" class="error">{error}</p>{/if}
  {#if processingError}<p role="alert" class="error">{processingError}</p>{/if}
  {#if message}<p class="message" role="status">{message}</p>{/if}
  {#if selected&&run}
    <section class="task-detail" aria-label="任务详情">
      <TaskRunDetail task={selected} {runId} panelOpen={runPanelOpen} bind:this={detail} onEditTask={()=>editTask(selected)} onCompleteTask={completeTask} onRestoreTask={createRun} {busy} onOpenRun={openRun} onProcessingFeedback={processingFeedback}/>
    </section>
  {:else}
    <TaskCollection bind:this={collection} tasks={filtered} total={tasks.length} {loading} {view} {status} {project} {sort} {busy} onSort={value=>sort=value} onSelect={select} onProcessingFeedback={processingFeedback}/>
  {/if}
</section>

<AppSidePanel open={runPanelOpen&&!!run} title={panelView==='changes'?'代码变更':run?.title??'执行详情'} description={selected?.title??'所选任务执行详情'} closeLabel="关闭执行详情" returnFocus={runTrigger} onClose={()=>runPanelOpen=false}>
  {#if run&&selected}
    <div bind:this={panelPages}>
    {#if error}<div class="task-surface"><p role="alert" class="error">{error}</p></div>{/if}
    {#if panelView==='changes'}
      <div class="task-surface"><button class="quiet panel-back" aria-label="返回执行详情" title="返回" on:click={backToRun}><ArrowLeft size={18}/></button></div>
      {#if changes}<TaskRunChanges taskId={selected.id} {run} {changes} {commits} loading={changesLoading} error={changesError}/>{/if}
    {/if}
    <div hidden={panelView!=='run'}>
      {#key run.id}<TaskRunContent bind:this={runContent} task={selected} {run} {busy} {changes} {changesLoading} onComplete={completeRun} onRestoreDevelopment={createRun} onOpenChat={openChat} onOpenChanges={openChanges}/>{/key}
    </div>
    </div>
  {/if}
</AppSidePanel>

<Dialog.Modal title={taskDraft ? '修改任务信息' : '新建任务'} class="ledger-dialog" bind:this={taskDialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={saveTask}><fieldset class="form-controls" disabled={busy}>
    <header><h3>{taskDraft ? '修改任务信息' : '新建任务'}</h3><button type="button" aria-label="关闭任务编辑" title="关闭" on:click={() => taskDialog.close()}><X size={18}/></button></header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <label>任务标题<input required maxlength="160" bind:value={taskTitle} placeholder="一句话描述要解决什么"/></label>
    <label>背景与现象<textarea rows="3" maxlength="8000" bind:value={taskDescription}></textarea></label>
    {#if !taskDraft}
      <label>工作区<div class="directory"><input required bind:value={workspace}/><button type="button" on:click={directory}>选择</button></div></label>
      <details><summary>首次执行与验收（可选）</summary>{@render runEditor()}</details>
    {/if}
    <footer><button class="primary">{busy ? '保存中…' : '确认'}</button></footer>
  </fieldset></form>
</Dialog.Modal>

<Dialog.Modal title={restoringDevelopment ? '改为开发中' : '新一次执行'} class="ledger-dialog" returnFocus={runReturnFocus} bind:this={runDialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={saveRun}><fieldset class="form-controls" disabled={busy}>
    <header><h3>{restoringDevelopment ? '改为开发中' : '新一次执行'}</h3><button type="button" aria-label="关闭执行编辑" title="关闭" on:click={() => runDialog.close()}><X size={18}/></button></header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <label>工作区<div class="directory"><input required bind:value={workspace}/><button type="button" on:click={directory}>选择</button></div></label>
    {@render runEditor()}
    <footer><small>新增执行，保留历史。</small><button class="primary" disabled={!runTitle.trim()}>{busy ? '保存中…' : '确认'}</button></footer>
  </fieldset></form>
</Dialog.Modal>

<TaskAgentSetup bind:this={setupDialog} returnFocus={moreTrigger}/>
