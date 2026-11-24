<script lang="ts">
  import {tick} from 'svelte';
  import {Check, Circle, ExternalLink, Link as LinkIcon, Pencil} from '@lucide/svelte';
  import TaskChangeCounts from './TaskChangeCounts.svelte';
  import {projectRunChanges,workspaceCommitState,type RunChanges} from './taskChanges';
  import {
    currentRun, statuses, actorLabel, results, scopes,
    criterionCheck, formatTaskTime as fmt,
    type Task, type Criterion
  } from './taskLedger';

  export let task: Task;
  export let runId = '';
  export let onEditTask: () => void;
  export let onVerify: (criterion: Criterion, returnFocus: HTMLElement) => void;
  export let onOpenChat: (conversationId: string) => void;
  export let onOpenChanges: (button:HTMLButtonElement)=>void=()=>{};
  export let changes:RunChanges|undefined=undefined;
  export let changesLoading=false;
  export let changesOpen=false;
  let heading: HTMLHeadingElement, navigation: HTMLElement, runScroll:HTMLDivElement;
  let expandedBackground = false, expandedGoal = false, expandedSolution = false;
  let shownTask = '', shownRun = '';
  $: changeSummary=changes??projectRunChanges(run.agentWorkspace,run.agentWorkspace.commitChanges??[]);
  $: run = task.runs.find(r => r.id === runId) ?? currentRun(task);
  $: if (shownTask !== task.id) { shownTask = task.id; expandedBackground = false; }
  $: if (shownRun !== run.id) { shownRun = run.id; expandedGoal = expandedSolution = false;runScroll?.scrollTo({top:0}); }
  $: writable = run.id === currentRun(task).id && run.status !== 'closed';
  $: codeState=workspaceCommitState(run.agentWorkspace);
  $: items = run.validationPlan.items;
  $: passed = items.filter(c => criterionCheck(run, c).passed).length;
  $: firstPending = items.findIndex(c => !criterionCheck(run, c).passed);

  export function focusHeading() { heading?.focus({preventScroll: true}); }
  async function choose(id: string) {
    runId = id;
    await tick();
    navigation?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({block: 'nearest', inline: 'nearest'});
  }
  async function navigate(event: KeyboardEvent) {
    if(!(event.target instanceof Element)||!event.target.closest('[role="tab"]'))return;
    const index = task.runs.findIndex(r => r.id === run.id);
    let next = index;
    if (event.key === 'ArrowRight') next = (index + 1) % task.runs.length;
    else if (event.key === 'ArrowLeft') next = (index - 1 + task.runs.length) % task.runs.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = task.runs.length - 1;
    else return;
    event.preventDefault();
    await choose(task.runs[next].id);
    navigation.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  }
  const clock = (n: number) => new Date(n).toLocaleTimeString('zh-CN', {hour: '2-digit', minute: '2-digit', hour12: false});
  function record(criterion: Criterion, button: HTMLButtonElement) {
    const summary = button.closest('details')?.querySelector('summary');
    if (summary) onVerify(criterion, summary);
  }
</script>

<div class="task-record">
  <div class="task-info" role="group" aria-label="任务信息">
  <header class="task-heading">
    <h3 tabindex="-1" bind:this={heading}>{task.title}</h3>
    <button class="quiet metadata-edit" aria-label="修改任务信息" title="修改任务信息" on:click={onEditTask}><Pencil size={15}/></button>
  </header>
  {#if task.description}
    <p class="task-background" class:clamped={!expandedBackground}>{task.description}</p>
    {#if task.description.length > 220}
      <button class="quiet text-toggle" aria-expanded={expandedBackground} on:click={() => expandedBackground = !expandedBackground}>{expandedBackground ? '收起任务背景' : '展开任务背景'}</button>
    {/if}
  {/if}
  <div class="task-context"><span>{task.projectName}</span><span>{actorLabel(task.createdBy)} 创建</span><time datetime={new Date(task.createdAt).toISOString()}>{fmt(task.createdAt)}</time></div>
  <hr class="task-divider"/>
  </div>
  <div class="task-run-scroll" bind:this={runScroll} role="region" aria-label="任务执行内容" tabindex="-1">

  <div class="run-navigation" role="tablist" tabindex="-1" aria-label="任务执行" bind:this={navigation} on:keydown={navigate}>
    {#each task.runs as item, index (item.id)}
      <div class="run-card-shell" class:selected={run.id===item.id}>
      <button role="tab" id={`run-tab-${item.id}`} aria-controls={`run-panel-${item.id}`}
        aria-selected={run.id === item.id} tabindex={run.id === item.id ? 0 : -1}
        class="run-card" title={item.title} class:selected={run.id === item.id} on:click={() => choose(item.id)}>
        <span class="run-title"><span class="run-number">{String(index + 1).padStart(2, '0')}</span><span>{item.title}</span></span>
        <span class="run-facts"><span>{item.kind}</span><span class="status-badge" data-status={item.status}>{statuses[item.status]}</span></span>
        <small>{actorLabel(item.createdBy)} · {fmt(item.createdAt)}</small>
      </button>
      <div class="card-conversations" aria-label="执行关联对话">
        {#each item.agentConversation as conversation}
          {#if conversation.agent==='codex'&&conversation.verified}
            <button class="quiet conversation-link" title={conversation.conversationId} aria-label={`打开 ${actorLabel(conversation.agent)} 对话 ${conversation.conversationId}`} on:click={()=>onOpenChat(conversation.conversationId)}><LinkIcon size={11}/><span>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}</span><ExternalLink size={10}/></button>
          {:else}<small title={conversation.conversationId}>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}（待核实）</small>{/if}
        {:else}<small>未关联对话</small>{/each}
      </div>
      </div>
    {/each}
  </div>

  <div class="run-content" role="tabpanel" id={`run-panel-${run.id}`} aria-labelledby={`run-tab-${run.id}`}>
    {#if run.id !== currentRun(task).id}<p class="muted history-note">历史执行，只读</p>{/if}
    <section aria-label="目标与方案" class="run-section">
      <h4>目标与方案</h4>
      <dl class="info-grid">
        <div><dt>目标</dt><dd><p class="goal" class:clamped={!expandedGoal}>{run.goal || '尚未记录本次目标'}</p>{#if run.goal.length > 220}<button class="quiet text-toggle" aria-expanded={expandedGoal} on:click={() => expandedGoal = !expandedGoal}>{expandedGoal ? '收起目标' : '展开目标'}</button>{/if}</dd></div>
        <div><dt>方案</dt><dd><p class:clamped={!expandedSolution}>{run.solution || '尚未记录方案'}</p>{#if run.solution.length > 220}<button class="quiet text-toggle" aria-expanded={expandedSolution} on:click={() => expandedSolution = !expandedSolution}>{expandedSolution ? '收起方案' : '展开方案'}</button>{/if}</dd></div>
      </dl>
      {#if run.description.trim() && ![run.goal.trim(), run.solution.trim()].includes(run.description.trim())}<details class="historical-description"><summary>处理说明</summary><p class="preserve">{run.description}</p></details>{/if}
      {#if run.closure}<p class="muted">{run.closure.accepted ? '人工验收通过' : '执行已结束'} · {run.closure.reason}</p>{/if}
    </section>

    <section aria-label="工作区" class="run-section workspace-section">
      <h4>工作区</h4>
      <dl class="info-grid">
        <div><dt>分支</dt><dd><code>{run.agentWorkspace.branch ?? '分支未知'}</code></dd></div>
        <div><dt>路径</dt><dd class="path">{run.agentWorkspace.path}</dd></div>
        <div><dt>变更</dt><dd class="code-summary">
          <span>{changesLoading?'读取变更…':changeSummary.complete?`${changeSummary.files.length} 个文件`:changeSummary.files.length?`${changeSummary.files.length} 个已知文件`:'变更待确认'}</span>
          <TaskChangeCounts changes={changeSummary} loading={changesLoading}/>
          {#if run.agentWorkspace.relatedCommits.length}<small>{run.agentWorkspace.relatedCommits.length} 个提交</small>{/if}
          <span class="code-state" data-state={codeState.state} title="提交状态来自工作区观测，推送状态基于本地上游引用">{codeState.label}</span>
          <button class="quiet text-link changes-toggle" aria-expanded={changesOpen} on:click={event=>onOpenChanges(event.currentTarget)}>查看变更</button>
        </dd></div>
      </dl>
    </section>

    <section aria-label="验收清单" class="run-section acceptance-section">
      <div class="section-heading"><h4>验收</h4><small>{passed} / {items.length} 通过</small></div>
      {#key `${task.id}-${run.id}`}
        <div class="criteria">
          {#each items as criterion, index (criterion.id)}
            {@const check = criterionCheck(run, criterion)}
            <details class="criterion" open={index === firstPending}>
              <summary aria-label={criterion.title}>
                <span class="check-mark" class:passed={check.passed} class:failed={check.result === 'failed'} aria-label={check.passed ? '通过' : check.result === 'failed' ? '失败' : '未验收'}>
                  {#if check.passed}<Check size={15} strokeWidth={2}/>{:else}<Circle size={13} strokeWidth={1.5}/>{/if}
                </span>
                <span class="criterion-title" title="验收点">{criterion.title}</span>
              </summary>
              <dl class="info-grid criterion-body">
                <div><dt>验收方式</dt><dd>{#if criterion.precondition}<p class="method-context">{criterion.precondition}</p>{/if}{criterion.steps}</dd></div>
                <div><dt>预期结果</dt><dd>{criterion.expected}</dd></div>
                <div><dt>验收结果</dt><dd>
                  <div class="criterion-result"><span class:failed={check.result === 'failed'}>{check.result === 'passed' ? `${scopes[check.stage]}${check.actor === 'agent' ? '自测' : '验收'}通过` : check.result === 'not_tested' ? '未验收' : results[check.result]}</span>{#if writable}<button class="quiet text-link record-result" on:click={event => record(criterion, event.currentTarget)}>记录结果</button>{/if}</div>
                  {#if check.evidence}<p class="result-conclusion">{check.evidence}</p>{/if}
                </dd></div>
              </dl>
            </details>
          {:else}<p class="muted">尚未登记验收清单</p>{/each}
        </div>
      {/key}
    </section>

    <section aria-label="活动时间线" class="run-section activity-section">
      <h4>活动</h4>
      <ol class="activity-timeline">
        {#each [...run.timeline].reverse() as entry (entry.id)}
          <li><Circle class="timeline-dot" size={6} fill="currentColor"/><time title={fmt(entry.at)} datetime={new Date(entry.at).toISOString()}>{clock(entry.at)}</time><div><p><span class="timeline-author">{actorLabel(entry.actor)}</span> {entry.summary}</p>{#if entry.reference}<p class="preserve muted">{entry.reference}</p>{/if}</div></li>
        {:else}<li><small>暂无活动</small></li>{/each}
      </ol>
    </section>
  </div>
  </div>
</div>

<style>
  .task-record { min-width: 0; min-height:0; height:100%;display:flex;flex-direction:column; }
  .task-info { flex:none;min-height:0;max-height:min(46%,280px);overflow:auto;scrollbar-width:none; }
  .task-info::-webkit-scrollbar { display:none; }
  .task-run-scroll { flex:1;min-height:0;min-width:0;overflow-y:auto;overflow-x:hidden;scrollbar-width:none;overscroll-behavior:contain;overflow-anchor:none;outline:none;padding:0 1px 12px 0; }
  .task-run-scroll::-webkit-scrollbar {display:none;width:0;height:0}
  .task-run-scroll:focus-visible {box-shadow:inset 2px 0 var(--color-focus-ring);}
  .task-heading { display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; }
  h3 { min-width: 0; flex: 1; margin: 0; font-size: 20px; line-height: 1.45; font-weight: 600; overflow-wrap: anywhere; }
  h3:focus { outline: none; }
  .task-record .metadata-edit { flex:none;display:grid;place-items:center;width:28px;height:28px;min-height:28px;padding:0;border:0;border-radius:5px; }
  .task-background { font-size: 14px; line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; margin: 6px 0 6px; }
  .task-context { display: flex; gap: 4px 8px; flex-wrap: wrap; color: var(--task-secondary); font-size: 12px; }
  .task-context > :not(:last-child)::after { content: ' ·'; margin-left: 8px; }
  .task-divider { border: 0; border-top: 1px solid var(--wb-border); margin: 12px 0 16px; }
  .run-navigation { display: flex; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .run-card-shell {flex:1 1 180px;min-width:0;max-width:250px;padding:8px 10px;background:var(--wb-panel);border:1px solid var(--wb-border);border-radius:6px;}
  .task-record .run-card {width:100%;min-width:0;display:flex;flex-direction:column;gap:2px;padding:0;text-align:left;border:0;border-radius:3px;background:transparent;min-height:0;}
  .run-card:focus-visible { outline-offset: -2px; }
  .run-card-shell.selected { background: var(--wb-control); border-color: color-mix(in srgb, var(--wb-border) 65%, var(--wb-border-strong)); }
  .run-title { min-width: 0; width: 100%; display: flex; gap: 8px; font-size: 14px; line-height: 1.5; color: var(--task-secondary); }
  .run-title > span:last-child { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .selected .run-title { color: var(--wb-text); font-weight: 500; }
  .run-number { flex: none; font-variant-numeric: tabular-nums; }
  .run-facts { display: flex; align-items: center; gap: 8px; color: var(--task-secondary); font-size: 12px; }
  .run-card .status-badge { padding: 0 4px; font-size: 12px; }
  .run-card small { display: block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; font-size: 12px; white-space: nowrap; }
  .run-card-shell:not(.selected) .status-badge { background: transparent; color: var(--task-secondary); font-weight: 400; padding: 0; }
  .run-section { margin-top: 20px; min-width: 0; }
  h4 { color: var(--wb-text); font-size: 15px; font-weight: 500; margin: 0 0 8px; }
  .info-grid { margin: 0; font-size: 14px; }
  .info-grid > div { display: grid; grid-template-columns: 60px minmax(0, 1fr); gap: 12px; margin: 6px 0; align-items: baseline; }
  dt { color: var(--task-secondary); font-size: 13px; white-space: nowrap; }
  dd { margin: 0; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  dd p { margin: 0; }
  .goal { font-weight: 500; }
  code, .path { font-family: var(--font-family-code); font-size: 13px; overflow-wrap: anywhere; }
  .code-summary {display:flex;gap:12px;align-items:baseline;flex-wrap:wrap;}
  .code-state {font-size:12px;white-space:nowrap;color:var(--task-secondary);}
  .code-state[data-state="dirty"] {color:var(--wb-warning-strong-text);}
  .code-state[data-state="unpushed"] {color:var(--wb-info-text);}
  .code-state[data-state="synced"] {color:var(--color-success);}
  .task-record .changes-toggle {padding:0;font-size:13px;min-height:22px;}
  .card-conversations {min-width:0;margin-top:4px;display:flex;flex-direction:column;align-items:flex-start;}
  .card-conversations small {font-size:12px;}
  .task-record .conversation-link {display:flex;align-items:center;gap:4px;max-width:100%;min-height:20px;padding:0;font-size:12px;border:0;color:var(--wb-info-text);}
  .conversation-link span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
  .historical-description { margin-top: 10px; font-size: 13px; color: var(--task-secondary); }
  .section-heading { display: flex; justify-content: space-between; align-items: baseline; gap: 10px; }
  .section-heading h4 { margin-bottom: 8px; }
  .criterion > summary { display: grid; grid-template-columns: 16px minmax(0, 1fr); gap: 10px; align-items: baseline; padding: 5px 0; cursor: pointer; list-style: none; }
  .criterion > summary::-webkit-details-marker { display: none; }
  .check-mark { align-self: center; display: flex; align-items: center; color: var(--task-secondary); }
  .check-mark.passed { color: var(--color-success); }
  .failed { color: var(--wb-danger-text); }
  .criterion-title { font-size: 14px; font-weight: 400; overflow-wrap: anywhere; }
  .criterion[open] .criterion-title { font-weight: 500; }
  .criterion-body { margin: 4px 0 10px 26px; }
  .criterion-result { display: flex; align-items: baseline; flex-wrap: wrap; gap: 12px; }
  .record-result { font-size: 13px; padding: 0; min-height: 24px; }
  .method-context {margin-bottom:4px;color:var(--task-secondary);}
  .result-conclusion {margin-top:4px;}
  .activity-timeline { list-style: none; padding: 0; margin: 0 0 0 5px; }
  .activity-timeline li { display: grid; grid-template-columns: 43px minmax(0, 1fr); gap: 12px; position: relative; border-left: 1px solid var(--wb-border); padding: 0 0 8px 16px; font-size: 13px; }
  .activity-timeline li:last-child { padding-bottom: 0; }
  .activity-timeline :global(.timeline-dot) { position: absolute; left: -3.5px; top: 6px; color: var(--task-secondary); }
  .activity-timeline time { color: var(--task-secondary); font-variant-numeric: tabular-nums; }
  .activity-timeline p { margin: 0 0 4px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .timeline-author { color: var(--task-secondary); }
  .history-note { margin-bottom: -10px; }
  @container (max-width: 700px) {
    .run-card-shell { flex-basis: 155px; }
    h3 { font-size: 18px; }
  }
</style>
