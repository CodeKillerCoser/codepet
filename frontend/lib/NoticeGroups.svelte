<script lang="ts">
  import NoticeActions from './NoticeActions.svelte';
  import { noticeStatusLabel, noticeNeedsInput } from './noticePresentation';

  import { groupActivities, type ActivityGroupId } from './activityGroups';
  import { type Notice, type NoticeSource } from './noticeGateway';
  import { runningBubbleStyle } from './gradientColor';
  import type { AppSettings } from './types';
  import MarkdownMessage from './MarkdownMessage.svelte';

  export let activities: Notice[];
  export let sources: NoticeSource[];
  export let runningBubble: AppSettings['appearance']['runningBubble'];
  export let dismiss: (event: MouseEvent, task: Notice) => void;
  export let expanded = new Set<ActivityGroupId>();
  $: groups = groupActivities(activities);
  $: bubbleStyle = runningBubbleStyle(runningBubble);

  function toggle(id: ActivityGroupId) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id); else next.add(id);
    expanded = next;
  }
  function expandFromCard(event: MouseEvent, id: ActivityGroupId) {
    if (!expanded.has(id) && !(event.target as Element).closest('button, a, .notice-actions')) toggle(id);
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
    <div class="activity-group-cards" role="group" aria-label={`${group.label}通知气泡`} id={`pet-group-${group.id}`}
      style={`--stack-count: ${Math.min(group.activities.length - 1, 2)}`}
      on:click={(event) => expandFromCard(event, group.id)}>
      {#each group.activities as activity, index (activity.noticeId)}
        <div class="activity-card-slot" class:stack-hidden={!isExpanded && index > 2}
          inert={!isExpanded && index > 0} aria-hidden={!isExpanded && index > 0 ? 'true' : undefined}
          style={`--stack-depth: ${index}; z-index: ${group.activities.length - index}`}>
          <article class="status-pill" class:active-status={!activity.finished}
            class:active-breath={!activity.finished && runningBubble.backgroundBreathing}
            class:active-marquee={!activity.finished && runningBubble.borderMarquee}
            class:urgent={noticeNeedsInput(activity)}
            class:done={activity.finished}
            style={!activity.finished ? bubbleStyle : undefined}>
            <div class="status-content">
              <div class="status-title-row">
                <span class="status-title"><span title={activity.content.title}>{activity.content.title}</span></span>
                <button class="dismiss-button" type="button" aria-label={`隐藏 ${activity.content.title}`}
                  on:click={(event) => dismiss(event, activity)}></button>
              </div>
              <MarkdownMessage compact message={activity.content.progress?.text?.trim() || '暂无进度说明'} />
              {#if activity.content.progress?.percent !== undefined}<progress max="100" value={activity.content.progress.percent} aria-label="通知进度"></progress>{/if}
              <NoticeActions notice={activity} available={sources.some(source => source.pluginId === activity.pluginId && source.enabled && source.connected)} />
              <div class="status-footer"><span class="status-meta">
                {sources.find(s => s.pluginId === activity.pluginId)?.displayName || activity.pluginId} · {noticeStatusLabel(activity)} ·
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
  progress { width:100%; height:4px; accent-color:var(--app-accent); }
  .status-title-row { grid-template-columns: minmax(0, 1fr) auto; }
  .status-title { min-width: 0; display: grid; font-size: var(--font-size-sm-plus); }
  .dismiss-button::before, .dismiss-button::after { left: 5px; top: 8px; }
</style>
