<script lang="ts">
  import { groupActivities, type ActivityGroupId } from './activityGroups';
  import { petStatusLabel, type PetTask, type PetSource } from './petGateway';
  import { runningBubbleStyle } from './gradientColor';
  import type { AppSettings } from './types';
  import MarkdownMessage from './MarkdownMessage.svelte';

  export let activities: PetTask[];
  export let sources: PetSource[];
  export let runningBubble: AppSettings['appearance']['runningBubble'];
  export let dismiss: (event: MouseEvent, task: PetTask) => void;
  let expanded = new Set<ActivityGroupId>();
  $: groups = groupActivities(activities);
  $: bubbleStyle = runningBubbleStyle(runningBubble);

  function toggle(id: ActivityGroupId) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id); else next.add(id);
    expanded = next;
  }
  function expandFromCard(event: MouseEvent, id: ActivityGroupId) {
    if (!expanded.has(id) && !(event.target as Element).closest('button, a')) toggle(id);
  }
</script>

{#each groups as group (group.id)}
  {@const isExpanded = expanded.has(group.id)}
  <section class={`activity-group group-${group.id}`} class:expanded={isExpanded} aria-label={group.label}>
    <button class="activity-group-toggle" type="button" aria-expanded={isExpanded}
      aria-controls={`pet-group-${group.id}`} on:click={() => toggle(group.id)}>
      <span class="activity-group-dot" aria-hidden="true"></span>
      <span>{group.label}</span><span class="activity-group-count">{group.activities.length}</span>
      <span class="activity-group-chevron" aria-hidden="true"></span>
    </button>
    <!-- The group button provides the keyboard equivalent of clicking the top bubble. -->
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div class="activity-group-cards" role="group" aria-label={`${group.label}任务气泡`} id={`pet-group-${group.id}`}
      style={`--stack-count: ${Math.min(group.activities.length - 1, 2)}`}
      on:click={(event) => expandFromCard(event, group.id)}>
      {#each group.activities as activity, index (activity.id)}
        <div class="activity-card-slot" class:stack-hidden={!isExpanded && index > 2}
          inert={!isExpanded && index > 0} aria-hidden={!isExpanded && index > 0 ? 'true' : undefined}
          style={`--stack-depth: ${index}; z-index: ${group.activities.length - index}`}>
          <article class="status-pill" class:active-status={activity.status === 'running'}
            class:active-breath={activity.status === 'running' && runningBubble.backgroundBreathing}
            class:active-marquee={activity.status === 'running' && runningBubble.borderMarquee}
            class:urgent={activity.status === 'waiting-approval' || activity.status === 'waiting-input'}
            class:failed={activity.status === 'failed'} class:done={activity.status === 'completed'}
            style={activity.status === 'running' ? bubbleStyle : undefined}>
            <div class="status-content">
              <div class="status-title-row">
                <span class="status-title"><span title={activity.title}>{activity.title}</span></span>
                <button class="dismiss-button" type="button" aria-label={`隐藏 ${activity.title}`}
                  on:click={(event) => dismiss(event, activity)}></button>
              </div>
              <MarkdownMessage compact message={activity.summary?.trim() || '正在思考中'} />
              <div class="status-footer"><span class="status-meta">
                {sources.find(s => s.id === activity.providerId)?.displayName || activity.providerId} · {petStatusLabel(activity.status)} ·
                <time class="status-ended-at" datetime={new Date(activity.updatedAt).toISOString()}>{new Date(activity.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
              </span></div>
            </div>
          </article>
        </div>
      {/each}
    </div>
  </section>
{/each}

<style>
  .status-title-row { grid-template-columns: minmax(0, 1fr) auto; }
  .status-title { min-width: 0; display: grid; font-size: var(--font-size-sm-plus); }
  .dismiss-button::before, .dismiss-button::after { left: 5px; top: 8px; }
</style>
