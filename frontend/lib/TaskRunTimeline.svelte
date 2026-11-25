<script lang="ts">
  import {Circle, MessageSquare, Reply} from '@lucide/svelte';
  import ExpandableText from './ExpandableText.svelte';
  import {actorLabel,formatTaskTime as fmt,type TaskRun} from './taskLedger';
  import {activityLabels,activityFields,activityValue,replyLabel} from './taskActivity';
  export let run:TaskRun;
  const clock=(at:number)=>new Date(at).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false});
  $: byId=new Map(run.timeline.map(entry=>[entry.id,entry]));
  function jump(id:string){const target=document.getElementById(`activity-${id}`);target?.scrollIntoView({block:'nearest'});target?.focus({preventScroll:true});}
</script>

<section aria-label="活动时间线" class="run-section activity-section">
  <h4>活动</h4>
  <ol class="activity-timeline">
    {#each [...run.timeline].reverse() as entry (entry.id)}
      <li id={`activity-${entry.id}`} tabindex="-1">
        {#if entry.kind==='comment'}<MessageSquare class="timeline-dot comment-dot" size={12}/>{:else}<Circle class="timeline-dot" size={6} fill="currentColor"/>{/if}
        <time title={fmt(entry.at)} datetime={new Date(entry.at).toISOString()}>{clock(entry.at)}</time>
        <div class="timeline-content">
          <div class="activity-meta"><strong>{actorLabel(entry.actor)}</strong><span>{activityLabels[entry.kind]??'活动'}</span>{#each entry.agentConversation??[] as conversation}<span title={conversation.conversationId}>{conversation.conversationId.slice(0,8)}</span>{/each}</div>
          {#if entry.replyTo}<button class="quiet reply-link" title={replyLabel(byId.get(entry.replyTo))} on:click={()=>jump(entry.replyTo!)}><Reply size={12}/><span>回复 {replyLabel(byId.get(entry.replyTo))}</span></button>{/if}
          <ExpandableText text={entry.summary} reference={entry.kind==='verification'?'':entry.reference??''} label="活动"/>
          {#if entry.changes?.length}<details class="activity-changes"><summary>查看变更</summary><dl>{#each entry.changes as change}<div><dt>{activityFields[change.field]??change.field}</dt><dd><div class="change-version"><span>之前</span><ExpandableText text={activityValue(change.field,change.before,run)} label="原内容"/></div><div class="change-version"><span>之后</span><ExpandableText text={activityValue(change.field,change.after,run)} label="新内容"/></div></dd></div>{/each}</dl></details>{/if}
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
 .activity-meta{display:flex;gap:var(--space-related);align-items:baseline;flex-wrap:wrap;color:var(--task-secondary);font:var(--font-caption);margin-bottom:var(--space-inline);}.activity-meta strong{font-weight:var(--font-weight-medium);color:var(--wb-text);}
 :global(.activity-timeline .comment-dot){left:-5px;top:7px;background:var(--wb-panel);}
 .timeline-content .reply-link{display:flex;align-items:center;gap:4px;max-width:100%;padding:2px 0;min-height:24px;margin-bottom:var(--space-inline);font:var(--font-caption);color:var(--task-secondary);border:0;background:transparent;text-align:left;}.reply-link span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}.reply-link :global(svg){flex:none;}
 .activity-changes{margin-top:var(--space-related);font:var(--font-caption);}.activity-changes summary{cursor:pointer;color:var(--task-secondary);}.activity-changes dl{margin:var(--space-related) 0;}.activity-changes dl>div{padding:var(--space-related) 0;border-top:1px solid var(--wb-border);}.activity-changes dt{font-weight:var(--font-weight-medium);margin-bottom:var(--space-inline);}.activity-changes dd{margin:0;}.change-version{padding:var(--space-inline) 0;}.change-version>span{color:var(--task-secondary);}
 li:focus-visible,summary:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:3px;}
 .timeline-content :global(.reading-toggle){min-height:20px;height:20px;margin-top:0;}
 .empty{display:block;font:var(--font-caption);color:var(--task-secondary);}
</style>
