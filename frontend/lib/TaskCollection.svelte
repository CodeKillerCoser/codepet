<script lang="ts">
  import {ArrowDown, ArrowUp, FolderClosed, Layers, ListChecks} from '@lucide/svelte';
  import TaskRunCounts from './TaskRunCounts.svelte';
  import TaskStatus from './TaskStatus.svelte';
  import {currentRun,criterionCheck,statuses,formatTaskTime as fmt,type Task,type RunStatus} from './taskLedger';
  export let tasks:Task[]=[];
  export let total=0;
  export let loading=false;
  export let view:'board'|'table'='board';
  export let status='all';
  export let project='';
  export let sort:'updated_desc'|'updated_asc'|'title'='updated_desc';
  export let onSort:(sort:'updated_desc'|'updated_asc'|'title')=>void;
  export let onSelect:(task:Task,button:HTMLButtonElement)=>void;
  let root:HTMLElement;
  const states:RunStatus[]=['in_progress','ready_for_delivery','published','closed'];
  $: ordered=[...tasks].sort((a,b)=>sort==='title'?a.title.localeCompare(b.title,'zh-CN'):((sort==='updated_asc'?1:-1)*(a.updatedAt-b.updatedAt)||a.id.localeCompare(b.id)));
  $: columns=status==='all'?states:states.filter(value=>value===status);
  $: showProject=!project&&new Set(tasks.map(task=>task.projectId)).size>1;
  const progress=(task:Task)=>{const run=currentRun(task);return {total:run.validationPlan.items.length,passed:run.validationPlan.items.filter(item=>criterionCheck(run,item).passed).length};};
  export function focusTask(id:string){root?.querySelector<HTMLButtonElement>(`[data-task-id="${CSS.escape(id)}"]`)?.focus({preventScroll:true});}
  function chronological(){onSort(sort==='updated_desc'?'updated_asc':'updated_desc');}
</script>

<section class="task-collection" aria-label={view==='board'?'任务看板':'任务表格'} aria-busy={loading} bind:this={root}>
 {#if loading&&!tasks.length}<div class="collection-empty" role="status">读取任务…</div>
 {:else if !tasks.length}
  <div class="collection-empty"><Layers size={25}/><h3>{total?'没有符合条件的任务':'还没有登记任务'}</h3><p>{total?'试试调整搜索或筛选条件。':'可以手动新建，也可以通过 MCP 登记开发任务。'}</p></div>
 {:else if view==='board'}
  <div class="task-board" class:single-column={columns.length===1}>
   {#each columns as state}
    {@const group=ordered.filter(task=>task.summary.status===state)}
    <section class="board-column" aria-label={statuses[state]}>
     <header class="board-heading"><h3><TaskStatus status={state} heading/><span class="column-count">{group.length}</span></h3></header>
     <div class="board-cards">
      {#each group as task (task.id)}
       {@const checks=progress(task)}
       <button class="task-row board-task" data-task-id={task.id} aria-label={task.title} on:click={event=>onSelect(task,event.currentTarget)}>
        {#if showProject}<span class="card-project"><FolderClosed size={12}/>{task.projectName}</span>{/if}
        <strong title={task.title}>{task.title}</strong>
        <TaskRunCounts summary={task.summary}/>
        <span class="card-summary"><span title="执行次数"><Layers size={12}/>{task.runs.length} 次执行</span>{#if checks.total}<span class="card-checks" title="当前执行验收进度"><ListChecks size={13}/>{checks.passed}/{checks.total}</span>{/if}<time title={fmt(task.updatedAt)} datetime={new Date(task.updatedAt).toISOString()}>{fmt(task.updatedAt)}</time></span>
       </button>
      {:else}<p class="column-empty">暂无任务</p>{/each}
     </div>
    </section>
   {/each}
  </div>
 {:else}
  <table class="task-table">
   <caption class="sr-only">任务列表，每行代表一个任务，状态和执行计数由全部执行汇总</caption>
   <colgroup><col class="title-column"/><col class="project-column"/><col class="status-column"/><col class="current-column"/><col class="runs-column"/><col class="checks-column"/><col class="time-column"/></colgroup>
   <thead><tr>
    <th scope="col" aria-sort={sort==='title'?'ascending':'none'}><button class="sort-heading" on:click={()=>onSort(sort==='title'?'updated_desc':'title')}>任务{#if sort==='title'}<ArrowUp size={12}/>{/if}</button></th>
    <th scope="col" class="project-column">项目</th><th scope="col">状态</th><th scope="col" class="current-column">当前执行</th><th scope="col" class="runs-column numeric">执行</th><th scope="col" class="checks-column numeric">验收</th>
    <th scope="col" class="time-column" aria-sort={sort==='title'?'none':sort==='updated_desc'?'descending':'ascending'}><button class="sort-heading" on:click={chronological}>最近活动{#if sort!=='title'}{#if sort==='updated_desc'}<ArrowDown size={12}/>{:else}<ArrowUp size={12}/>{/if}{/if}</button></th>
   </tr></thead>
   <tbody>
    {#each ordered as task (task.id)}
     {@const run=currentRun(task)}{@const checks=progress(task)}
     <tr class="task-table-row">
      <th scope="row"><button class="task-row table-task" data-task-id={task.id} title={task.title} on:click={event=>onSelect(task,event.currentTarget)}>{task.title}</button></th>
      <td class="project-column"><span title={task.projectName}>{task.projectName}</span></td>
      <td class="task-state-cell"><TaskStatus status={task.summary.status}/><TaskRunCounts summary={task.summary}/></td>
      <td class="current-column"><span title={run.title}>{run.title}</span></td>
      <td class="runs-column numeric">{task.runs.length}</td><td class="checks-column numeric">{checks.total?`${checks.passed}/${checks.total}`:'—'}</td>
      <td class="time-column"><time title={fmt(task.updatedAt)} datetime={new Date(task.updatedAt).toISOString()}>{fmt(task.updatedAt)}</time></td>
     </tr>
    {/each}
   </tbody>
  </table>
 {/if}
</section>

<style>
 .task-collection{flex:1;min-width:0;min-height:0;overflow:auto;overscroll-behavior:contain;padding:18px 0 10px;}
 .task-board{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:16px;min-height:100%;align-items:start;}
 .task-board.single-column{grid-template-columns:minmax(240px,380px);}
 .board-column{min-width:0;min-height:170px;padding:0 4px 12px;}
 .board-heading{height:40px;display:flex;align-items:center;padding:0 7px;}
 h3{margin:0;font-size:var(--font-size-label);font-weight:var(--font-weight-medium);}
 .board-heading h3{display:flex;align-items:center;gap:9px;}
 .column-count{font-size:var(--font-size-caption);color:var(--task-secondary);font-weight:var(--font-weight-regular);}
 .board-cards{display:flex;flex-direction:column;gap:8px;}
 .task-collection .board-task{display:flex;flex-direction:column;align-items:stretch;width:100%;gap:10px;padding:15px 14px;text-align:left;background:var(--wb-panel);border:1px solid var(--wb-border);border-radius:7px;}
 .task-collection .board-task:hover{border-color:var(--wb-border-strong);}
 .board-task strong{font-size:var(--font-size-body);line-height:var(--line-height-body-relaxed);font-weight:var(--font-weight-medium);overflow-wrap:anywhere;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-clamp:2;overflow:hidden;}
 .card-project{display:flex;align-items:center;gap:5px;font-size:var(--font-size-caption);color:var(--task-secondary);}
 .card-summary{display:flex;align-items:center;gap:6px 10px;flex-wrap:wrap;color:var(--task-secondary);font-size:var(--font-size-caption);margin-top:3px;}
 .card-summary>span{display:inline-flex;align-items:center;gap:4px;}
 .card-summary time{margin-left:auto;font-size:var(--font-size-caption);font-variant-numeric:tabular-nums;}
 .column-empty{font-size:var(--font-size-caption);text-align:center;color:var(--task-secondary);margin:28px 0 20px;}
 .task-table{width:100%;border-collapse:collapse;table-layout:fixed;font-size:var(--font-size-body);}
 .title-column{width:26%;}.project-column{width:10%;}.status-column{width:19%;}.current-column{width:16%;}.runs-column{width:6%;}.checks-column{width:7%;}.time-column{width:16%;}
 .task-table th,.task-table td{text-align:left;border-bottom:1px solid var(--wb-border);padding:10px 12px;min-width:0;vertical-align:middle;}
 .task-table thead{position:sticky;top:-18px;z-index:1;background:var(--wb-panel);}
 .task-table thead th{color:var(--task-secondary);font-weight:var(--font-weight-emphasis);font-size:var(--font-size-label);height:38px;}
 .task-table tbody tr{height:48px;}
 .task-table tbody tr:hover,.task-table tbody tr:focus-within{background:var(--wb-control);}
 .task-table th:first-child{padding-left:4px;}
 .task-collection .table-task{display:block;min-width:0;width:100%;padding:2px 0;text-align:left;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:var(--font-size-body);font-weight:var(--font-weight-medium);border:0;border-radius:3px;background:transparent;}
 .task-collection .table-task:hover{color:var(--wb-text);background:transparent;}
 td>span{display:block;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}
 .task-table .numeric{text-align:right;font-variant-numeric:tabular-nums;}
 .task-state-cell :global(.run-counts){margin-top:var(--space-inline);}
 .time-column time{font-size:var(--font-size-caption);white-space:nowrap;color:var(--task-secondary);font-variant-numeric:tabular-nums;}
 .task-collection .sort-heading{display:inline-flex;align-items:center;gap:5px;min-height:24px;padding:0;border:0;background:transparent;font-size:var(--font-size-caption);color:var(--task-secondary);}
 .collection-empty{min-height:220px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;text-align:center;color:var(--task-secondary);padding:30px;}
 .collection-empty h3{font-size:var(--font-size-subheading);color:var(--wb-text);}.collection-empty p{font-size:var(--font-size-caption);margin:0;}
 @container(max-width:1050px){.task-board{gap:12px;}.current-column{display:none;}.title-column{width:30%;}.project-column{width:14%;}.status-column{width:20%;}.runs-column{width:7%;}.checks-column{width:9%;}.time-column{width:20%;}}
 @container(max-width:850px){.project-column{display:none;}.title-column{width:36%;}.status-column{width:22%;}.runs-column{width:8%;}.checks-column{width:10%;}.time-column{width:24%;}}
 @container(max-width:520px){.time-column,.checks-column{display:none;}.title-column{width:53%;}.status-column{width:37%;}.runs-column{width:10%;}.task-table th,.task-table td{padding:10px 5px;}.task-table :global(.task-status){font-size:var(--font-size-caption);gap:4px;}.board-task strong{font-size:var(--font-size-body);}}
</style>
