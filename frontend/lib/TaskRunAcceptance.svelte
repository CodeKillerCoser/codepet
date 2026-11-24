<script lang="ts">
  import {Check,Circle,X} from '@lucide/svelte';
  import {criterionCheck,type TaskRun} from './taskLedger';
  export let run:TaskRun;
  $: items=run.validationPlan.items;
  $: passed=items.filter(item=>criterionCheck(run,item).passed).length;
  $: firstPending=items.findIndex(item=>!criterionCheck(run,item).passed);
</script>

<section aria-label="验收清单" class="run-section acceptance-section">
  <header><h4>验收</h4><span class="acceptance-count">{passed} / {items.length} 通过</span></header>
  <div class="criteria">
    {#each items as criterion,index (criterion.id)}
      {@const check=criterionCheck(run,criterion)}
      <details class="criterion" open={index===firstPending}>
        <summary aria-label={criterion.title}>
          <span class="check-mark" class:passed={check.passed} class:failed={check.result==='failed'} aria-label={check.passed?'通过':check.result==='failed'?'失败':'未验收'}>
            {#if check.passed}<Check size={15}/>{:else if check.result==='failed'}<X size={15}/>{:else}<Circle size={13} strokeWidth={1.5}/>{/if}
          </span>
          <span class="criterion-title">{criterion.title}</span>
        </summary>
        <dl class="criterion-body">
          <div><dt>验收方式</dt><dd>{#if criterion.precondition}<p>{criterion.precondition}</p>{/if}{criterion.steps}</dd></div>
          <div><dt>预期结果</dt><dd>{criterion.expected}</dd></div>
          <div><dt>验收结果</dt><dd>{check.evidence||'暂无验收结论'}</dd></div>
        </dl>
      </details>
    {:else}<p class="empty">尚未登记验收清单</p>{/each}
  </div>
</section>

<style>
 .acceptance-section{min-width:0;margin-top:var(--space-section);}
 header{display:flex;justify-content:space-between;align-items:baseline;gap:var(--space-related);margin-bottom:var(--space-related);}
 h4{font:var(--font-subheading);margin:0;}
 .acceptance-count{font:var(--font-caption);color:var(--task-secondary);white-space:nowrap;}
 .criterion{min-width:0;}
 summary{display:grid;grid-template-columns:16px minmax(0,1fr);gap:var(--space-related);align-items:start;padding:var(--space-related) 0;list-style:none;cursor:pointer;}
 summary::-webkit-details-marker{display:none;}
 .check-mark{display:flex;align-items:center;height:calc(var(--font-size-body)*var(--line-height-body));color:var(--task-secondary);}
 .check-mark.passed{color:var(--color-success);}.check-mark.failed{color:var(--wb-danger-text);}
 .criterion-title{font:var(--font-body);font-weight:var(--font-weight-emphasis);overflow-wrap:anywhere;}
 .criterion-body{display:grid;gap:var(--space-related);margin:0 0 var(--space-block) 24px;padding:var(--space-inline) 0 var(--space-related);}
 .criterion-body>div{display:grid;grid-template-columns:4.5em minmax(0,1fr);gap:var(--space-related);align-items:baseline;min-width:0;}
 dt{font:var(--font-caption);color:var(--task-secondary);}
 dd{min-width:0;margin:0;font:var(--font-body);white-space:pre-wrap;overflow-wrap:anywhere;}
 dd p{margin:0 0 var(--space-inline);}
 .empty{font:var(--font-caption);color:var(--task-secondary);}
 @media(max-width:360px){.criterion-body>div{grid-template-columns:minmax(0,1fr);gap:var(--space-inline);}}
</style>
