<script lang="ts">
  import {Circle} from '@lucide/svelte';
  import ExpandableText from './ExpandableText.svelte';
  import {actorLabel,formatTaskTime as fmt,type TaskRun} from './taskLedger';
  export let run:TaskRun;
  const clock=(at:number)=>new Date(at).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false});
</script>

<section aria-label="活动时间线" class="run-section activity-section">
  <h4>活动</h4>
  <ol class="activity-timeline">
    {#each [...run.timeline].reverse() as entry (entry.id)}
      <li>
        <Circle class="timeline-dot" size={6} fill="currentColor"/>
        <time title={fmt(entry.at)} datetime={new Date(entry.at).toISOString()}>{clock(entry.at)}</time>
        <div class="timeline-content">
          <ExpandableText text={entry.summary} prefix={actorLabel(entry.actor)} reference={entry.reference??''} label="活动"/>
        </div>
      </li>
    {:else}<li class="empty">暂无活动</li>{/each}
  </ol>
</section>

<style>
 .activity-section{min-width:0;margin-top:var(--space-section);}
 h4{font:var(--font-subheading);margin:0 0 var(--space-block);}
 .activity-timeline{list-style:none;padding:0;margin:0 0 0 5px;}
 li{position:relative;display:grid;grid-template-columns:3.2em minmax(0,1fr);gap:var(--space-related);padding:0 0 var(--space-related) 16px;min-width:0;font:var(--font-body);}
 li::before{content:'';position:absolute;left:0;top:12px;bottom:-12px;border-left:1px solid var(--wb-border);}
 li:last-child::before{display:none;}li:last-child{padding-bottom:0;}
 :global(.activity-timeline .timeline-dot){position:absolute;left:-2.5px;top:9px;color:var(--task-secondary);}
 time{font:var(--font-caption);line-height:calc(var(--font-size-body)*var(--line-height-body));white-space:nowrap;font-variant-numeric:tabular-nums;color:var(--task-secondary);}
 .timeline-content{min-width:0;}
 .timeline-content :global(.reading-toggle){min-height:20px;height:20px;margin-top:0;}
 .empty{display:block;font:var(--font-caption);color:var(--task-secondary);}
</style>
