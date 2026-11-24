<script lang="ts">
  import {ChevronRight, Pencil, CheckCheck, ChevronDown, ChevronUp} from '@lucide/svelte';
  import TaskStatus from './TaskStatus.svelte';
  import {currentRun,kinds,actorLabel,criterionCheck,formatTaskTime as fmt,type Task} from './taskLedger';

  export let task:Task;
  export let runId='';
  export let panelOpen=false;
  export let busy=false;
  export let onEditTask:()=>void;
  export let onOpenRun:(id:string,button:HTMLButtonElement)=>void=()=>{};
  let heading:HTMLHeadingElement;
  let expandedBackground=false,shownTask='';
  $: if(shownTask!==task.id){shownTask=task.id;expandedBackground=false;}
  $: executions=task.runs.map((run,index)=>({run,index})).reverse();
  export function focusHeading(){heading?.focus({preventScroll:true});}
  function navigate(event:KeyboardEvent){
    if(!(event.target instanceof HTMLButtonElement)||!event.target.matches('.run-card')||!(event.currentTarget instanceof HTMLElement))return;
    const cards=[...event.currentTarget.querySelectorAll<HTMLButtonElement>('.run-card')];
    const index=cards.indexOf(event.target);
    let next=index;
    if(event.key==='ArrowDown')next=(index+1)%cards.length;
    else if(event.key==='ArrowUp')next=(index-1+cards.length)%cards.length;
    else if(event.key==='Home')next=0;
    else if(event.key==='End')next=cards.length-1;
    else return;
    event.preventDefault();cards[next]?.focus();
  }
</script>

<div class="task-record">
  <div class="task-info" role="group" aria-label="任务信息">
    <header class="task-heading">
      <h3 tabindex="-1" bind:this={heading}>{task.title}</h3>
      <button class="quiet metadata-edit" disabled={busy} aria-label="修改任务信息" title="修改任务信息" on:click={onEditTask}><Pencil size={17}/></button>
    </header>
    {#if task.description}
      <p class="task-background" class:clamped={!expandedBackground}>{task.description}</p>
      {#if task.description.length>220}<button class="quiet text-toggle" aria-label={expandedBackground?'收起任务背景':'展开任务背景'} title={expandedBackground?'收起':'展开'} aria-expanded={expandedBackground} on:click={()=>expandedBackground=!expandedBackground}>{#if expandedBackground}<ChevronUp size={16}/>{:else}<ChevronDown size={16}/>{/if}</button>{/if}
    {/if}
    <div class="task-context"><span>{task.projectName}</span><span>{actorLabel(task.createdBy)} 创建</span><time datetime={new Date(task.createdAt).toISOString()}>{fmt(task.createdAt)}</time></div>
    <hr class="task-divider"/>
  </div>
  <div class="task-run-scroll" role="region" aria-label="任务执行列表" tabindex="-1">
    <div class="executions-heading"><h4>任务执行 <span>{task.runs.length}</span></h4></div>
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions (Arrow keys supplement the ordinary execution buttons.) -->
    <div class="run-navigation" role="group" aria-label="任务执行" on:keydown={navigate}>
      {#each executions as {run,index} (run.id)}
        {@const passed=run.validationPlan.items.filter(c=>criterionCheck(run,c).passed).length}
        <button class="run-card" class:selected={panelOpen&&runId===run.id} id={`run-tab-${run.id}`}
          aria-expanded={panelOpen&&runId===run.id} aria-controls={panelOpen&&runId===run.id?`run-panel-${run.id}`:undefined}
          on:click={event=>onOpenRun(run.id,event.currentTarget)}>
          <span class="run-number">{String(index+1).padStart(2,'0')}</span>
          <span class="run-summary">
            <span class="run-title"><strong>{run.title}</strong>{#if run.id===currentRun(task).id}<small class="current-label">当前执行</small>{/if}</span>
            {#if run.goal.trim() && run.goal.trim()!==run.title.trim()}<span class="run-preview">{run.goal}</span>{/if}
            <span class="run-facts"><span>{kinds[run.kind]}</span><span>{actorLabel(run.createdBy)}</span><time datetime={new Date(run.updatedAt).toISOString()}>{fmt(run.updatedAt)}</time>
              {#if run.validationPlan.items.length}<span class="run-validation"><CheckCheck size={13}/>{passed}/{run.validationPlan.items.length}</span>{/if}
            </span>
          </span>
          <span class="run-state"><TaskStatus status={run.status}/><ChevronRight size={17}/></span>
        </button>
      {/each}
    </div>
  </div>
</div>

<style>
  .task-record{height:100%;min-width:0;min-height:0;display:flex;flex-direction:column;}
  .task-info{flex:none;max-height:min(48%,300px);overflow:auto;scrollbar-width:none;}
  .task-info::-webkit-scrollbar{display:none;}
  .task-heading{display:flex;align-items:flex-start;gap:16px;}
  h3{flex:1;min-width:0;margin:0;font:var(--font-title);letter-spacing:var(--letter-spacing-title);overflow-wrap:anywhere;}
  h3:focus{outline:none;}
  .task-record .metadata-edit{display:grid;place-items:center;flex:none;min-height:32px;width:32px;padding:0;}
  .task-background{font-size:var(--font-size-body);line-height:var(--line-height-reading-spacious);margin:12px 0;color:var(--task-secondary);white-space:pre-wrap;overflow-wrap:anywhere;}
  .task-context{display:flex;gap:6px 16px;flex-wrap:wrap;font-size:var(--font-size-caption);color:var(--task-secondary);margin-top:14px;}
  .task-context > :not(:last-child)::after{content:'·';margin-left:16px;color:var(--wb-border-strong);}
  .task-divider{border:0;border-top:1px solid var(--wb-border);margin:22px 0 0;}
  .task-run-scroll{flex:1;min-height:0;overflow-y:auto;overflow-x:hidden;scrollbar-width:none;overscroll-behavior:contain;overflow-anchor:none;outline:none;padding:20px 1px 12px;}
  .task-run-scroll::-webkit-scrollbar{display:none;}
  .task-run-scroll:focus-visible{box-shadow:inset 2px 0 var(--color-focus-ring);}
  .executions-heading{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:14px;}
  h4{margin:0;font:var(--font-section);}
  h4 span{margin-left:6px;font-size:var(--font-size-caption);font-weight:var(--font-weight-regular);color:var(--task-secondary);}
  .run-navigation{display:flex;flex-direction:column;}
  .task-record .run-card{display:flex;align-items:flex-start;gap:14px;min-width:0;width:100%;padding:18px 10px;border:0;border-bottom:1px solid var(--wb-border);border-radius:0;background:transparent;text-align:left;}
  .task-record .run-card:hover{background:var(--wb-control);border-color:var(--wb-border-strong);}
  .task-record .run-card.selected{background:var(--wb-control);border-color:var(--wb-border-strong);}
  .run-number{flex:none;min-width:22px;padding-top:1px;color:var(--task-secondary);font-size:var(--font-size-caption);font-variant-numeric:tabular-nums;}
  .run-summary{flex:1;min-width:0;}
  .run-title{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;}
  .run-title strong{font:var(--font-subheading);overflow-wrap:anywhere;}
  .current-label{white-space:nowrap;}
  .run-preview{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-clamp:2;overflow:hidden;font-size:var(--font-size-label);line-height:var(--line-height-reading);color:var(--task-secondary);margin-top:5px;overflow-wrap:anywhere;}
  .run-facts{display:flex;align-items:center;gap:6px 14px;flex-wrap:wrap;font-size:var(--font-size-caption);color:var(--task-secondary);margin-top:10px;}
  .run-validation{display:inline-flex;align-items:center;gap:4px;min-width:0;max-width:100%;}
  .run-state{display:flex;align-items:center;gap:10px;flex:none;padding-top:2px;color:var(--task-secondary);}
  @container(max-width:600px){
    h3{font-size:var(--font-size-title-compact);}
    .task-record .run-card{gap:10px;padding:14px 12px;flex-wrap:wrap;}
    .run-state{margin-left:32px;padding-top:0;}
    .run-title strong{font-size:var(--font-size-body);}
  }
</style>
