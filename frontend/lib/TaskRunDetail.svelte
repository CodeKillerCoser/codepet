<script lang="ts">
  import {tick} from 'svelte';
  import {Check, Circle, ExternalLink, Link as LinkIcon} from '@lucide/svelte';
  import {
    currentRun, statuses, actorLabel, results, scopes, fileLabels,
    criterionCheck, criterionRecords, formatTaskTime as fmt,
    type Task, type TaskRun, type Criterion
  } from './taskLedger';

  export let task: Task;
  export let runId = '';
  export let onEditTask: () => void;
  export let onVerify: (criterion: Criterion, returnFocus: HTMLElement) => void;
  export let onOpenChat: (conversationId: string) => void;
  let heading: HTMLHeadingElement, navigation: HTMLElement;
  let expandedBackground = false, expandedGoal = false, expandedSolution = false;
  let shownTask = '', shownRun = '', showFiles = false;
  $: run = task.runs.find(r => r.id === runId) ?? currentRun(task);
  $: if (shownTask !== task.id) { shownTask = task.id; expandedBackground = false; }
  $: if (shownRun !== run.id) { shownRun = run.id; expandedGoal = expandedSolution = false; showFiles = false; }
  $: writable = run.id === currentRun(task).id && run.status !== 'closed';
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
  <header class="task-heading">
    <h3 tabindex="-1" bind:this={heading}>{task.title}</h3>
    <button class="quiet metadata-edit" on:click={onEditTask}>修改任务信息</button>
  </header>
  {#if task.description}
    <p class="task-background" class:clamped={!expandedBackground}>{task.description}</p>
    {#if task.description.length > 220}
      <button class="quiet text-toggle" aria-expanded={expandedBackground} on:click={() => expandedBackground = !expandedBackground}>{expandedBackground ? '收起任务背景' : '展开任务背景'}</button>
    {/if}
  {/if}
  <div class="task-context"><span>{task.projectName}</span><span>{actorLabel(task.createdBy)} 创建</span><time datetime={new Date(task.createdAt).toISOString()}>{fmt(task.createdAt)}</time></div>
  <hr class="task-divider"/>

  <div class="run-navigation" role="tablist" tabindex="-1" aria-label="任务执行" bind:this={navigation} on:keydown={navigate}>
    {#each task.runs as item, index (item.id)}
      <button role="tab" id={`run-tab-${item.id}`} aria-controls={`run-panel-${item.id}`}
        aria-selected={run.id === item.id} tabindex={run.id === item.id ? 0 : -1}
        class="run-card" title={item.title} class:selected={run.id === item.id} on:click={() => choose(item.id)}>
        <span class="run-title"><span class="run-number">{String(index + 1).padStart(2, '0')}</span><span>{item.title}</span></span>
        <span class="run-facts"><span>{item.kind}</span><span class="status-badge" data-status={item.status}>{statuses[item.status]}</span></span>
        <small>{actorLabel(item.createdBy)} · {fmt(item.createdAt)}</small>
      </button>
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
      <div class="agent-conversations" aria-label="关联对话">
        {#each run.agentConversation as conversation}
          {#if conversation.agent === 'codex' && conversation.verified}
            <button class="quiet text-link conversation-link" on:click={() => onOpenChat(conversation.conversationId)}><LinkIcon size={13}/>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0, 8)}<ExternalLink size={11}/></button>
          {:else}<small>{actorLabel(conversation.agent)} · {conversation.conversationId}（待核实）</small>{/if}
        {:else}<small>尚未关联 Agent 对话</small>{/each}
      </div>
      {#if run.description}<details class="historical-description"><summary>补充说明</summary><p class="preserve">{run.description}</p></details>{/if}
      {#if run.closure}<p class="muted">{run.closure.accepted ? '人工验收通过' : '执行已结束'} · {run.closure.reason}</p>{/if}
    </section>

    <section aria-label="工作区" class="run-section workspace-section">
      <h4>工作区</h4>
      <dl class="info-grid">
        <div><dt>分支</dt><dd><code>{run.agentWorkspace.branch ?? '分支未知'}</code><small class="workspace-state">{run.agentWorkspace.changeStatus.hasUncommittedChanges === null ? '改动状态未知' : run.agentWorkspace.changeStatus.hasUncommittedChanges ? `${run.agentWorkspace.changedFiles.length} 个文件未提交` : '工作区干净'}</small></dd></div>
        <div><dt>路径</dt><dd class="path">{run.agentWorkspace.path}</dd></div>
        <div><dt>提交</dt><dd><span>{run.agentWorkspace.relatedCommits.length} 个关联提交</span><button class="quiet text-link changes-toggle" aria-expanded={showFiles} aria-controls={`run-files-${run.id}`} on:click={() => showFiles = !showFiles}>查看变更</button></dd></div>
      </dl>
      <div class="workspace-changes" id={`run-files-${run.id}`} hidden={!showFiles}>
        <small>观测于 {fmt(run.agentWorkspace.observedAt)}；{run.agentWorkspace.changeStatus.hasUnpushedCommits === null ? '推送状态未知' : run.agentWorkspace.changeStatus.hasUnpushedCommits ? '有待推送提交' : '无待推送提交'}。文件由同目录任务共享，推送状态基于本地上游引用。</small>
        {#if run.agentWorkspace.error}<p class="muted">{run.agentWorkspace.error}</p>{/if}
        <div class="changed-files">{#each run.agentWorkspace.changedFiles as file}<div class="changed-file"><small>{fileLabels[file.changeType] ?? file.changeType}{file.staged ? ' · 已暂存' : ''}</small><code>{file.originalPath ? `${file.originalPath} → ` : ''}{file.path}</code></div>{:else}<p class="muted">没有记录到变更文件</p>{/each}</div>
        {#each run.agentWorkspace.relatedCommits as sha}<code class="commit">{sha}</code>{/each}
      </div>
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
                <span class="criterion-title">{criterion.title}</span>
              </summary>
              <dl class="info-grid criterion-body">
                {#if criterion.precondition}<div><dt>前提</dt><dd>{criterion.precondition}</dd></div>{/if}
                <div><dt>操作</dt><dd>{criterion.steps}</dd></div>
                <div><dt>预期</dt><dd>{criterion.expected}</dd></div>
                <div><dt>验收结果</dt><dd class="criterion-result"><span class:failed={check.result === 'failed'}>{check.passed ? '通过' : check.result === 'passed' ? '自测通过，待人工验收' : check.result === 'not_tested' ? '未验收' : results[check.result]}</span>{#if writable}<button class="quiet text-link record-result" on:click={event => record(criterion, event.currentTarget)}>记录结果</button>{/if}</dd></div>
                {#if check.evidence}<div><dt>说明</dt><dd>{check.evidence}</dd></div>{/if}
              </dl>
              {#if criterionRecords(run, criterion.id).length}
                <details class="criterion-history"><summary>验收记录</summary>
                  {#each criterionRecords(run, criterion.id) as record}
                    {@const result = record.results.find(r => r.criterionId === criterion.id)!}
                    <div class="verification-record"><small>{actorLabel(record.agent)}{record.actor === 'agent' ? ' 自测' : ''} · {scopes[record.stage]} · {fmt(record.createdAt)} · 清单第 {record.planRevision} 版</small><p>{record.target} · {record.environment} · {results[result.result]}</p><p class="preserve">{result.evidence}</p></div>
                  {/each}
                </details>
              {/if}
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

<style>
  .task-record { min-width: 0; }
  .task-heading { display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; }
  h3 { min-width: 0; flex: 1; margin: 0; font-size: 20px; line-height: 1.45; font-weight: 600; overflow-wrap: anywhere; }
  h3:focus { outline: none; }
  .task-record .metadata-edit { flex: none; font-size: 10px; padding: 2px 0; min-height: 22px; }
  .task-background { font-size: 12px; line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; margin: 6px 0 6px; }
  .task-context { display: flex; gap: 4px 8px; flex-wrap: wrap; color: var(--wb-muted); font-size: 10px; }
  .task-context > :not(:last-child)::after { content: ' ·'; margin-left: 8px; }
  .task-divider { border: 0; border-top: 1px solid var(--wb-border); margin: 12px 0 16px; }
  .run-navigation { display: flex; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .run-card { flex: 1 1 150px; min-width: 0; max-width: 240px; display: flex; flex-direction: column; gap: 2px; padding: 8px 12px; text-align: left; background: var(--wb-panel); border: 1px solid var(--wb-border); border-radius: 6px; }
  .run-card:focus-visible { outline-offset: -2px; }
  .run-card.selected { background: var(--wb-control); border-color: color-mix(in srgb, var(--wb-border) 65%, var(--wb-border-strong)); }
  .run-title { min-width: 0; width: 100%; display: flex; gap: 8px; font-size: 12px; line-height: 1.5; color: var(--wb-muted); }
  .run-title > span:last-child { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .selected .run-title { color: var(--wb-text); font-weight: 500; }
  .run-number { flex: none; font-variant-numeric: tabular-nums; }
  .run-facts { display: flex; align-items: center; gap: 8px; color: var(--wb-muted); font-size: 10px; }
  .run-card .status-badge { padding: 0 4px; font-size: 10px; }
  .run-card small { display: block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; font-size: 10px; white-space: nowrap; }
  .run-card:not(.selected) .status-badge { background: transparent; color: var(--wb-muted); font-weight: 400; padding: 0; }
  .run-section { margin-top: 20px; min-width: 0; }
  h4 { color: var(--wb-muted); font-size: 13px; font-weight: 500; margin: 0 0 8px; }
  .info-grid { margin: 0; font-size: 12px; }
  .info-grid > div { display: grid; grid-template-columns: 44px minmax(0, 1fr); gap: 12px; margin: 6px 0; align-items: baseline; }
  dt { color: var(--wb-muted); font-size: 11px; white-space: nowrap; }
  dd { margin: 0; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  dd p { margin: 0; }
  .goal { font-weight: 500; }
  code, .path { font-family: var(--font-family-code); font-size: 11px; overflow-wrap: anywhere; }
  .workspace-state { margin-left: 12px; }
  .workspace-changes { margin-top: 8px; font-size: 11px; }
  .task-record .changes-toggle { margin-left: 12px; padding: 0; font-size: 11px; min-height: 22px; }
  .changed-files { margin-top: 8px; }
  .changed-file { display: grid; grid-template-columns: 80px minmax(0, 1fr); gap: 8px; padding: 4px 0; }
  .commit { display: block; margin-top: 5px; }
  .agent-conversations { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; margin-top: 6px; overflow-wrap: anywhere; }
  .task-record .conversation-link { display: inline-flex; align-items: center; gap: 5px; padding: 0; font-size: 11px; min-height: 22px; }
  .historical-description { margin-top: 10px; font-size: 11px; color: var(--wb-muted); }
  .section-heading { display: flex; justify-content: space-between; align-items: baseline; gap: 10px; }
  .section-heading h4 { margin-bottom: 8px; }
  .criterion > summary { display: grid; grid-template-columns: 16px minmax(0, 1fr); gap: 10px; align-items: baseline; padding: 5px 0; cursor: pointer; list-style: none; }
  .criterion > summary::-webkit-details-marker { display: none; }
  .check-mark { align-self: center; display: flex; align-items: center; color: var(--wb-muted); }
  .check-mark.passed { color: var(--wb-success-text); }
  .failed { color: var(--wb-danger-text); }
  .criterion-title { font-size: 12px; font-weight: 400; overflow-wrap: anywhere; }
  .criterion[open] .criterion-title { font-weight: 500; }
  .criterion-body { margin: 4px 0 10px 26px; }
  .criterion-result { display: flex; align-items: baseline; flex-wrap: wrap; gap: 12px; }
  .record-result { font-size: 11px; padding: 0; min-height: 24px; }
  .criterion-history { margin: 0 0 12px 26px; font-size: 11px; color: var(--wb-muted); }
  .verification-record { margin-top: 10px; }
  .verification-record p { margin: 4px 0; }
  .activity-timeline { list-style: none; padding: 0; margin: 0 0 0 5px; }
  .activity-timeline li { display: grid; grid-template-columns: 43px minmax(0, 1fr); gap: 12px; position: relative; border-left: 1px solid var(--wb-border); padding: 0 0 8px 16px; font-size: 11px; }
  .activity-timeline li:last-child { padding-bottom: 0; }
  .activity-timeline :global(.timeline-dot) { position: absolute; left: -3.5px; top: 6px; color: var(--wb-muted); }
  .activity-timeline time { color: var(--wb-muted); font-variant-numeric: tabular-nums; }
  .activity-timeline p { margin: 0 0 4px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .timeline-author { color: var(--wb-muted); }
  .history-note { margin-bottom: -10px; }
  @container (max-width: 400px) {
    .workspace-state { display: block; margin-left: 0; }
  }
  @container (max-width: 700px) {
    .run-card { flex-basis: 155px; }
    .task-heading { flex-wrap: wrap; }
    h3 { font-size: 18px; }
  }
</style>
