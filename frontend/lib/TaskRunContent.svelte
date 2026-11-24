<script lang="ts">
  import {ExternalLink, Link as LinkIcon, ChevronDown, ChevronUp} from '@lucide/svelte';
  import TaskRunStatus from './TaskRunStatus.svelte';
  import TaskRunAcceptance from './TaskRunAcceptance.svelte';
  import TaskRunTimeline from './TaskRunTimeline.svelte';
  import TaskWorkspaceStatus from './TaskWorkspaceStatus.svelte';
  import TaskChangeCounts from './TaskChangeCounts.svelte';
  import {projectRunChanges,type RunChanges} from './taskChanges';
  import {currentRun,kinds,actorLabel,formatTaskTime as fmt,type Task,type TaskRun} from './taskLedger';
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
</script>
<div class="task-run-panel task-surface" aria-label="任务执行详情">
  <div class="execution-meta"><TaskRunStatus status={run.status} current={run.id===currentRun(task).id} {busy} {onComplete} onRestore={onRestoreDevelopment}/><span>{kinds[run.kind]}</span><span>{actorLabel(run.createdBy)} · {fmt(run.createdAt)}</span></div>
  <div class="run-content" id={`run-panel-${run.id}`}>
    {#if run.id !== currentRun(task).id}<p class="muted history-note">历史执行，只读</p>{/if}
    <section aria-label="目标与方案" class="run-section">
      <h4>目标与方案</h4>
      <dl class="info-grid">
        <div><dt>目标</dt><dd><p class="goal" class:clamped={!expandedGoal}>{run.goal || '尚未记录本次目标'}</p>{#if run.goal.length > 220}<button class="quiet text-toggle" aria-label={expandedGoal?'收起目标':'展开目标'} aria-expanded={expandedGoal} on:click={() => expandedGoal = !expandedGoal}>{#if expandedGoal}<ChevronUp size={16}/>{:else}<ChevronDown size={16}/>{/if}</button>{/if}</dd></div>
        <div><dt>方案</dt><dd><p class:clamped={!expandedSolution}>{run.solution || '尚未记录方案'}</p>{#if run.solution.length > 220}<button class="quiet text-toggle" aria-label={expandedSolution?'收起方案':'展开方案'} aria-expanded={expandedSolution} on:click={() => expandedSolution = !expandedSolution}>{#if expandedSolution}<ChevronUp size={16}/>{:else}<ChevronDown size={16}/>{/if}</button>{/if}</dd></div>
      </dl>

      <div class="panel-conversations" aria-label="执行关联对话">
        {#each run.agentConversation as conversation}
          {#if conversation.agent==='codex'&&conversation.verified}
            <button class="quiet conversation-link" title={conversation.conversationId} on:click={()=>onOpenChat(conversation.conversationId)} aria-label={`打开 ${actorLabel(conversation.agent)} 对话 ${conversation.conversationId}`}><LinkIcon size={14}/><span>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}</span><ExternalLink size={12}/></button>
          {:else}<small>{actorLabel(conversation.agent)} · {conversation.conversationId.slice(0,8)}（待核实）</small>{/if}
        {/each}
      </div>
      {#if run.description.trim() && ![run.goal.trim(), run.solution.trim()].includes(run.description.trim())}<details class="historical-description"><summary>处理说明</summary><p class="preserve">{run.description}</p></details>{/if}
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
          <TaskWorkspaceStatus workspace={run.agentWorkspace}/>
          <button bind:this={changesTrigger} class="quiet text-link changes-toggle" on:click={event=>onOpenChanges(event.currentTarget)}>查看变更</button>
        </dd></div>
      </dl>
    </section>

    <TaskRunAcceptance {run}/>
    <TaskRunTimeline {run}/>
  </div>

</div>

<style>
  .task-run-panel{min-width:0;color:var(--wb-text);font:var(--font-body);}
  .execution-meta{display:flex;align-items:center;gap:8px 12px;flex-wrap:wrap;font-size:var(--font-size-caption);color:var(--task-secondary);}
  .run-section{min-width:0;margin-top:var(--space-section);}
  h4{color:var(--wb-text);font:var(--font-subheading);margin:0 0 var(--space-related);}
  .info-grid{margin:0;font-size:var(--font-size-body);}
  .info-grid>div{display:grid;grid-template-columns:2.8em minmax(0,1fr);gap:var(--space-related);align-items:baseline;margin:var(--space-related) 0;}
  dt{font-size:var(--font-size-caption);color:var(--task-secondary);margin-bottom:0;}
  dd{margin:0;min-width:0;white-space:pre-wrap;overflow-wrap:anywhere;}
  dd p{margin:0;}
  code,.path{font:var(--font-size-caption)/var(--line-height-reading) var(--font-family-code);overflow-wrap:anywhere;}
  .workspace-section .info-grid>div{grid-template-columns:2.8em minmax(0,1fr);}
  .code-summary{display:flex;gap:8px 12px;align-items:baseline;flex-wrap:wrap;}
  .task-run-panel .changes-toggle{padding:0;min-height:24px;font-size:var(--font-size-label);}
  .panel-conversations{margin-top:var(--space-related);}
  .task-run-panel .conversation-link{display:inline-flex;align-items:center;gap:6px;font-size:var(--font-size-caption);padding:2px 0;min-height:28px;color:var(--wb-info-text);}
  .historical-description{margin-top:var(--space-related);font-size:var(--font-size-label);color:var(--task-secondary);}
  .history-note{font-size:var(--font-size-caption);margin:14px 0 0;}
</style>
