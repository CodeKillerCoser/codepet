<script lang="ts">
  import {Check, Circle, ExternalLink, Link as LinkIcon, RotateCcw} from '@lucide/svelte';
  import TaskChangeCounts from './TaskChangeCounts.svelte';
  import {projectRunChanges,workspaceCommitState,type RunChanges} from './taskChanges';
  import {currentRun,statuses,kinds,actorLabel,criterionCheck,formatTaskTime as fmt,type Task,type TaskRun} from './taskLedger';
  export let task:Task;
  export let run:TaskRun;
  export let busy=false;
  export let onComplete:()=>void;
  export let onRestoreDevelopment:(button:HTMLButtonElement)=>void;
  export let onOpenChat:(id:string)=>void;
  export let onOpenChanges:(button:HTMLButtonElement)=>void;
  export let changes:RunChanges|undefined=undefined;
  export let changesLoading=false;
  let expandedGoal=false,expandedSolution=false;
  let changesTrigger:HTMLButtonElement;
  export function focusChanges(){changesTrigger?.focus({preventScroll:true});}
  $: changeSummary=changes??projectRunChanges(run.agentWorkspace,run.agentWorkspace.commitChanges??[]);
  $: codeState=workspaceCommitState(run.agentWorkspace);
  $: items=run.validationPlan.items;
  $: passed=items.filter(c=>criterionCheck(run,c).passed).length;
  $: firstPending=items.findIndex(c=>!criterionCheck(run,c).passed);
  const clock=(n:number)=>new Date(n).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false});
</script>
<div class="task-run-panel task-surface" aria-label="任务执行详情">
  <div class="execution-meta"><span class="status-badge" data-status={run.status}>{statuses[run.status]}</span><span>{kinds[run.kind]}</span><span>{actorLabel(run.createdBy)} · {fmt(run.createdAt)}</span></div>
  {#if run.id===currentRun(task).id && ['ready_for_delivery','closed'].includes(run.status)}
    <footer class="execution-actions">
      {#if run.status==='ready_for_delivery'}<button class="complete-run" disabled={busy} on:click={onComplete}><Check size={16}/>{busy?'保存中…':'标记完成'}</button>
      {:else}<button class="complete-run" disabled={busy} on:click={event=>onRestoreDevelopment(event.currentTarget)}><RotateCcw size={16}/>改为开发中</button>{/if}
    </footer>
  {/if}
  <div class="run-content" id={`run-panel-${run.id}`}>
    {#if run.id !== currentRun(task).id}<p class="muted history-note">历史执行，只读</p>{/if}
    <section aria-label="目标与方案" class="run-section">
      <h4>目标与方案</h4>
      <dl class="info-grid">
        <div><dt>目标</dt><dd><p class="goal" class:clamped={!expandedGoal}>{run.goal || '尚未记录本次目标'}</p>{#if run.goal.length > 220}<button class="quiet text-toggle" aria-expanded={expandedGoal} on:click={() => expandedGoal = !expandedGoal}>{expandedGoal ? '收起目标' : '展开目标'}</button>{/if}</dd></div>
        <div><dt>方案</dt><dd><p class:clamped={!expandedSolution}>{run.solution || '尚未记录方案'}</p>{#if run.solution.length > 220}<button class="quiet text-toggle" aria-expanded={expandedSolution} on:click={() => expandedSolution = !expandedSolution}>{expandedSolution ? '收起方案' : '展开方案'}</button>{/if}</dd></div>
      </dl>

      <div class="panel-conversations" aria-label="执行关联对话">
        {#each run.agentConversation as conversation}
          {#if conversation.agent==='codex'&&conversation.verified}
            <button class="quiet conversation-link" title={conversation.conversationId} on:click={()=>onOpenChat(conversation.conversationId)} aria-label={`打开 ${actorLabel(conversation.agent)} 对话 ${conversation.conversationId}`}><LinkIcon size={14}/><span>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}</span><ExternalLink size={12}/></button>
          {:else}<small>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}（待核实）</small>{/if}
        {/each}
      </div>
      {#if run.description.trim() && ![run.goal.trim(), run.solution.trim()].includes(run.description.trim())}<details class="historical-description"><summary>处理说明</summary><p class="preserve">{run.description}</p></details>{/if}
      {#if run.closure}<p class="muted">{#if run.closure.accepted}人工验收通过 · {/if}{run.closure.reason}</p>{/if}
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
          <button bind:this={changesTrigger} class="quiet text-link changes-toggle" on:click={event=>onOpenChanges(event.currentTarget)}>查看变更</button>
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
                <div><dt>验收结果</dt><dd>{check.evidence || '暂无验收结论'}</dd></div>
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

<style>
  .task-run-panel{min-width:0;color:var(--wb-text);font:15px/1.75 var(--font-family-ui);}
  .execution-meta{display:flex;align-items:center;gap:8px 12px;flex-wrap:wrap;font-size:13px;color:var(--task-secondary);}
  .execution-actions{margin-top:12px;}
  .task-run-panel .complete-run{display:inline-flex;align-items:center;gap:6px;padding:6px 10px;font-size:14px;}
  .run-section{min-width:0;margin-top:26px;}
  h4{color:var(--wb-text);font-size:16px;font-weight:600;margin:0 0 12px;}
  .info-grid{margin:0;font-size:15px;}
  .info-grid>div{margin:12px 0;}
  dt{font-size:13px;color:var(--task-secondary);margin-bottom:3px;}
  dd{margin:0;min-width:0;white-space:pre-wrap;overflow-wrap:anywhere;}
  dd p{margin:0;}
  .goal{font-weight:500;}
  code,.path{font:13px/1.7 var(--font-family-code);overflow-wrap:anywhere;}
  .workspace-section .info-grid>div{display:grid;grid-template-columns:42px minmax(0,1fr);gap:10px;align-items:baseline;}
  .code-summary{display:flex;gap:8px 12px;align-items:baseline;flex-wrap:wrap;}
  .code-state{font-size:13px;color:var(--task-secondary);}
  .code-state[data-state="dirty"]{color:var(--wb-warning-strong-text);}
  .code-state[data-state="unpushed"]{color:var(--wb-info-text);}
  .code-state[data-state="synced"]{color:var(--color-success);}
  .task-run-panel .changes-toggle{padding:0;min-height:24px;font-size:14px;}
  .panel-conversations{margin-top:12px;}
  .task-run-panel .conversation-link{display:inline-flex;align-items:center;gap:6px;font-size:13px;padding:2px 0;min-height:28px;color:var(--wb-info-text);}
  .historical-description{margin-top:14px;font-size:14px;color:var(--task-secondary);}
  .section-heading{display:flex;justify-content:space-between;align-items:baseline;gap:12px;}
  .criterion>summary{display:grid;grid-template-columns:16px minmax(0,1fr);gap:12px;align-items:baseline;padding:9px 0;cursor:pointer;list-style:none;}
  .criterion>summary::-webkit-details-marker{display:none;}
  .check-mark{align-self:center;display:flex;align-items:center;color:var(--task-secondary);}
  .check-mark.passed{color:var(--color-success);}
  .check-mark.failed{color:var(--wb-danger-text);}
  .criterion-title{font-size:15px;font-weight:500;line-height:1.65;overflow-wrap:anywhere;}
  .criterion-body{padding:2px 12px 10px 14px;margin:0 0 8px 26px;border-left:2px solid var(--wb-border);}
  .method-context{margin-bottom:4px;color:var(--task-secondary);}
  .activity-timeline{list-style:none;padding:0;margin:0 0 0 5px;}
  .activity-timeline li{position:relative;display:grid;grid-template-columns:40px minmax(0,1fr);gap:10px;border-left:1px solid var(--wb-border);padding:0 0 16px 16px;font-size:14px;}
  .activity-timeline li:last-child{padding-bottom:0;}
  .activity-timeline :global(.timeline-dot){position:absolute;left:-3.5px;top:8px;color:var(--task-secondary);}
  .activity-timeline time{color:var(--task-secondary);font-variant-numeric:tabular-nums;}
  .activity-timeline p{margin:0 0 6px;white-space:pre-wrap;overflow-wrap:anywhere;}
  .timeline-author{color:var(--task-secondary);}
  .history-note{font-size:13px;margin:14px 0 0;}
</style>
