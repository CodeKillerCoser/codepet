<script lang="ts">
  import { Bell, ExternalLink, PanelTop, Plug, X } from '@lucide/svelte';
  import PetAvatar from './PetAvatar.svelte';
  import MarkdownMessage from '../MarkdownMessage.svelte';
  import DockPanel from './DockPanel.svelte';
  import type { PetSurfaceController } from './surface';
  import { petStatusLabel, type PetTask } from '../petGateway';
  import type { AppSettings, TaskStatus } from '../types';
  import type { PetData } from './model';
  import type { SurfaceView } from './surface';
  import type { petSizing } from './sizing';
  export let sizing: ReturnType<typeof petSizing>;
  export let data: PetData;
  export let settings: AppSettings | null;
  export let avatarStatus: TaskStatus;
  export let surface: SurfaceView;
  export let controller: PetSurfaceController | null;
  export let pointer: { x: number; y: number } | null;
  export let failed: (error: unknown) => void;
  export let actionNotice: string;
  export let dragWindow: () => Promise<void>;
  export let toggleSurface: (event: MouseEvent) => void;
  export let openMain: (event: MouseEvent) => void;
  export let dismissActivity: (event: MouseEvent, task: PetTask) => void;
  let failedIcons = new Set<string>();
  let expandedCompleted = new Set<string>();
  let stackSelection = new Map<string, string>();
  const wheelProgress = new Map<string, { delta: number; at: number; flippedAt: number }>();

  const attention = (task: PetTask) => ['waiting-approval', 'waiting-input'].includes(task.status);
  const live = (task: PetTask) => task.status === 'running' || attention(task);
  const completedOrCancelled = (task: PetTask) => task.status === 'completed' || task.status === 'interrupted';
  function completedByProvider(tasks: PetTask[]) {
    const groups = new Map<string, { id: string; providerId: string; completed: boolean; tasks: PetTask[] }>();
    for (const task of tasks) {
      if (!completedOrCancelled(task)) continue;
      const providerId = task.providerId || '';
      const id = `completed-${encodeURIComponent(providerId)}`;
      if (!groups.has(id)) groups.set(id, { id, providerId, completed: true, tasks: [] });
      groups.get(id)!.tasks.push(task);
    }
    return [...groups.values()];
  }
  $: completedGroups = completedByProvider(data.tasks);
  $: taskGroups = [
    { id: 'active', providerId: '', completed: false, tasks: data.tasks.filter(task => !completedOrCancelled(task)).sort((a, b) => Number(live(b)) - Number(live(a))) },
    ...completedGroups,
  ].filter(group => group.tasks.length > 0);
  $: if ([...expandedCompleted].some(id => !completedGroups.some(group => group.id === id))) {
    expandedCompleted = new Set([...expandedCompleted].filter(id => completedGroups.some(group => group.id === id)));
  }
  $: if ([...stackSelection].some(([id, taskId]) => !completedGroups.some(group => group.id === id && group.tasks.some(task => task.id === taskId)))) {
    stackSelection = new Map([...stackSelection].filter(([id, taskId]) => completedGroups.some(group => group.id === id && group.tasks.some(task => task.id === taskId))));
  }
  $: for (const id of wheelProgress.keys()) {
    if (!completedGroups.some(group => group.id === id)) wheelProgress.delete(id);
  }
  function stackTasks(tasks: PetTask[], selectedId: string | undefined) {
    const start = Math.max(0, tasks.findIndex(task => task.id === selectedId));
    return Array.from({ length: Math.min(tasks.length, 3) }, (_, index) => tasks[(start + index) % tasks.length]);
  }
  function rotateStack(event: WheelEvent, id: string, tasks: PetTask[]) {
    // Preserve pinch zoom and horizontal gestures. Expanded/single rows never call this.
    if (event.ctrlKey || !event.deltaY || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
    event.preventDefault();
    event.stopPropagation();
    const now = performance.now();
    const previous = wheelProgress.get(id);
    const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 240 : 1);
    // Trackpad micro-deltas accumulate, while momentum cannot race through the pile.
    if (previous && now - previous.flippedAt < 160) return;
    const accumulated = previous && now - previous.at < 180 && Math.sign(previous.delta) === Math.sign(delta)
      ? previous.delta + delta : delta;
    if (Math.abs(accumulated) < 40) {
      wheelProgress.set(id, { delta: accumulated, at: now, flippedAt: previous?.flippedAt ?? -Infinity });
      return;
    }
    const current = Math.max(0, tasks.findIndex(task => task.id === stackSelection.get(id)));
    const next = (current + Math.sign(accumulated) + tasks.length) % tasks.length;
    stackSelection = new Map(stackSelection).set(id, tasks[next].id);
    wheelProgress.set(id, { delta: 0, at: now, flippedAt: now });
  }
  function toggleCompleted(id: string) {
    wheelProgress.delete(id);
    const next = new Set(expandedCompleted);
    if (next.has(id)) next.delete(id); else next.add(id);
    expandedCompleted = next;
  }
  $: latest = data.tasks[0];
  function expandCompleted(event: MouseEvent, id: string) {
    if (!expandedCompleted.has(id) && !(event.target as Element).closest('button, a')) toggleCompleted(id);
  }
</script>

<DockPanel {controller} {surface} {pointer} {sizing} {dragWindow} {failed}
  title={data.error || (latest ? `${petStatusLabel(latest.status)} · ${latest.title}` : '就绪')}>
  <svelte:fragment slot="leading">
    <PetAvatar petId={settings?.pet.selectedPetId ?? 'default'} kind={settings?.pet.kind} imagePath={settings?.pet.imagePath}
      portrait width={sizing.portraitWidth} interactive label="桌宠头像" status={avatarStatus} dragging={surface.dragging} />
  </svelte:fragment>
  <span slot="indicator" class="status-dot" class:busy={latest?.status === 'running'} class:attention={latest && attention(latest)} class:error={Boolean(data.error) || latest?.status === 'failed'}></span>
  <span slot="status" class="latest-status">{data.error ? '连接中断' : latest ? petStatusLabel(latest.status) : '就绪'}</span>
  <span slot="summary" class="latest-title">{data.error ? '连接中断' : latest?.title || '就绪'}</span>
  <svelte:fragment slot="trailing">
    {#if data.tasks.length > 1}<span class="task-count" aria-label={`${data.tasks.length} 项任务`}>{data.tasks.length}</span>{/if}
  </svelte:fragment>
  <svelte:fragment slot="actions">
    <button class="panel-action" type="button" aria-label="切换为悬浮" title="切换为悬浮" on:click={toggleSurface}><PanelTop size={14} /></button>
    <button class="panel-action" type="button" aria-label="打开主窗口" title="打开主窗口" on:click={openMain}><ExternalLink size={14} /></button>
  </svelte:fragment>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="notch-task-list" tabindex="0" role="region" aria-label="任务列表">
        {#if data.error}<p class="connection-error" role="status">活动连接暂不可用，正在重试</p>{/if}
        {#each taskGroups as group (group.id)}
          {@const completedExpanded = expandedCompleted.has(group.id)}
          {@const stacked = group.completed && !completedExpanded && group.tasks.length > 1}
          {#if group.completed && group.id === completedGroups[0]?.id}
            <h2 class="completed-heading">已完成 <span class="completed-count">{completedGroups.reduce((count, item) => count + item.tasks.length, 0)}</span></h2>
          {/if}
          <!-- The first provider icon toggles its group with pointer or keyboard. -->
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
          <div class="task-group" class:stacked class:completed={group.completed} role="group" aria-label={group.completed ? `${group.providerId || 'Agent'} 已完成任务` : '当前任务'}
            id={`notch-${group.id}`}
            style={`--stack-count: ${Math.min(group.tasks.length - 1, 2)}`}
            on:click={event => { if (group.completed) expandCompleted(event, group.id); }}
            on:wheel|nonpassive={event => { if (stacked) rotateStack(event, group.id, group.tasks); }}>
            {#if group.completed}
              {@const provider = data.providers.find(item => item.providerId === group.providerId)}
              {@const providerName = provider?.displayName || data.sources.find(source => source.id === group.providerId)?.displayName || group.providerId || 'Agent'}
              <div class="provider-icon group-icon" title={providerName}>
                <button class="completed-toggle" type="button" aria-expanded={completedExpanded}
                  aria-label={`${providerName} 已完成任务（${group.tasks.length}）`}
                  title={`${completedExpanded ? '收起' : '展开'} ${providerName} 的已完成任务`}
                  aria-controls={`notch-${group.id}-tasks`} on:click={() => toggleCompleted(group.id)}>
                  {#if provider?.icon && !failedIcons.has(provider.icon)}
                    <img src={provider.icon} alt={providerName} referrerpolicy="no-referrer" on:error={() => { failedIcons = new Set([...failedIcons, provider.icon!]); }} />
                  {:else}<Plug size={sizing.portraitWidth * 2 / 3} aria-label={providerName} />{/if}
                </button>
              </div>
            {/if}
            <div class="task-group-items" id={`notch-${group.id}-tasks`}>
              {#each stacked ? stackTasks(group.tasks, stackSelection.get(group.id)) : group.tasks as task, index (task.id)}
                {@const provider = data.providers.find(item => item.providerId === task.providerId)}
                {@const providerName = provider?.displayName || data.sources.find(source => source.id === task.providerId)?.displayName || task.providerId || 'Agent'}
                <article class="notch-task" class:stack-back={stacked && index > 0}
                  inert={stacked && index > 0} aria-hidden={stacked && index > 0 ? 'true' : undefined}
                  style={stacked ? `--stack-depth: ${index}; z-index: ${3 - index}` : undefined}
                  aria-label={`${task.title}，${petStatusLabel(task.status)}`}>
                  {#if !group.completed}
                    <div class="provider-icon" title={providerName}>
                      {#if provider?.icon && !failedIcons.has(provider.icon)}
                        <img src={provider.icon} alt={providerName} referrerpolicy="no-referrer" on:error={() => { failedIcons = new Set([...failedIcons, provider.icon!]); }} />
                      {:else}<Plug size={sizing.portraitWidth * 2 / 3} aria-label={providerName} />{/if}
                    </div>
                  {/if}
                  <div class="task-content">
                    <div class="task-topline">
                      <h3 title={task.title}>{task.title}</h3>
                      <span class="task-state" class:running={task.status === 'running'} class:attention={attention(task)} class:failed={task.status === 'failed'}>{petStatusLabel(task.status)}</span>
                      <time class="task-time" datetime={new Date(task.updatedAt).toISOString()} title={`最近更新：${new Date(task.updatedAt).toLocaleString()}`} aria-label={`最近更新 ${new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`}>{new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
                      {#if attention(task)}<span class="task-attention" title={petStatusLabel(task.status)}><Bell size={13} aria-label={petStatusLabel(task.status)} /></span>{/if}
                    </div>
                    <div class="task-progress"><MarkdownMessage compact message={task.summary?.trim() || '尚未收到回复内容'} /></div>
                    <button class="dismiss-task" type="button" aria-label={`隐藏 ${task.title}`} title="隐藏任务" on:click={event => dismissActivity(event, task)}><X size={13} /></button>
                  </div>
                </article>
              {/each}
            </div>
          </div>
        {:else}
          <div class="empty-state"><strong>暂时没有任务</strong><span>开始编码后，最新进展会显示在这里</span></div>
        {/each}
      </div>
      {#if actionNotice}<p class="action-notice" role="status">{actionNotice}</p>{/if}
</DockPanel>

<style>
  button { color: inherit; border: 0; background: transparent; }
  button:focus-visible, .notch-task-list:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: -2px; border-radius: 6px; }
  .panel-action { display: grid; place-items: center; flex: none; width: 22px; height: 22px; border-radius: 6px; color: var(--gray-10); }
  .status-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--green-9); flex: none; }
  .status-dot.busy { background: var(--blue-9); animation: pulse 1.6s ease-in-out infinite; }
  .status-dot.attention { background: var(--amber-9); }
  .status-dot.error, .connection-error { color: var(--red-11); }
  .status-dot.error { background: var(--red-9); }
  .latest-title { display: block; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
  .task-count { color: var(--gray-11); font-size: 11px; font-variant-numeric: tabular-nums; flex: none; }
  .notch-task-list { container-type: inline-size; min-height: 0; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin; scrollbar-color: var(--gray-7) transparent; padding: 0; }
  .task-group { min-width: 0; }
  .task-group.completed { display: grid; grid-template-columns: var(--task-icon-size) minmax(0, 1fr); gap: 2px; align-items: start; }
  .completed .group-icon { position: relative; top: 6px; }
  .task-group-items { min-width: 0; display: grid; gap: 12px; }
  .completed .notch-task { display: flex; flex-direction: column; align-items: stretch; }
  .notch-task { position: relative; min-width: 0; display: grid; grid-template-columns: var(--task-icon-size) minmax(0, 1fr); gap: 8px; align-items: start; }
  .completed-heading { margin: 12px 0 8px; color: var(--gray-11); font-size: 11px; font-weight: 400; line-height: 20px; }
  .completed-count { margin-left: 4px; color: var(--gray-10); font-variant-numeric: tabular-nums; }
  .completed-toggle { display: grid; place-items: center; width: 100%; height: 100%; padding: 0; border-radius: 4px; cursor: pointer; }
  .completed + .completed { margin-top: 12px; }
  .task-group.stacked { --stack-step: 11px; cursor: pointer; }
  .stacked .task-group-items { grid-template-columns: minmax(0, 1fr); gap: 0; isolation: isolate; padding-bottom: calc(var(--stack-count) * var(--stack-step) + 1px); }
  .stacked .notch-task { grid-area: 1 / 1; perspective: 600px; perspective-origin: 50% 0; }
  .stacked .stack-back { pointer-events: none; }
  .completed .task-content::before { content: ''; position: absolute; inset: 0; z-index: -1; background: color-mix(in srgb, var(--color-black) 20%, var(--gray-2)); border-radius: 6px; }
  .stacked .notch-task:not(.stack-back) .task-content::before { box-shadow: inset 0 0 0 1px var(--gray-4); }
  .stacked .stack-back .task-content::before { background: var(--gray-3); box-shadow: inset 0 -1px 0 var(--gray-6); }
  .stacked .stack-back:last-child .task-content::before { background: color-mix(in srgb, var(--color-black) 20%, var(--gray-3)); }
  .completed .task-content { flex: 1; isolation: isolate; padding: 6px; }
  .completed .task-topline { padding-right: 0; }
  .completed .notch-task:hover .task-topline, .completed .notch-task:focus-within .task-topline { padding-right: 18px; }
  .completed .dismiss-task { top: 8px; right: 6px; }
  .stacked .task-content { align-self: stretch; transform: translate3d(0, calc(var(--stack-depth) * var(--stack-step)), calc(var(--stack-depth) * -18px)); transform-origin: center top; transition: transform 140ms ease-out; }
  .stacked .stack-back .task-content > * { visibility: hidden; }
  .provider-icon { margin-top: max(0px, calc((24px - var(--task-icon-size)) / 2)); width: var(--task-icon-size); height: var(--task-icon-size); display: grid; place-items: center; color: var(--gray-11); }
  .provider-icon img { width: var(--task-icon-size); height: var(--task-icon-size); object-fit: contain; border-radius: 4px; }
  .task-content { position: relative; min-width: 0; padding: 0 4px 4px 0; }
  .task-topline { display: flex; align-items: center; gap: 8px; min-height: 24px; padding-right: 18px; }
  h3 { flex: 1; min-width: 0; margin: 0; font-size: 14px; font-weight: 600; line-height: 1.5; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .latest-status, .task-state, .task-time { flex: none; white-space: nowrap; font-size: 10px; color: var(--gray-11); }
  .latest-status { font-size: 11px; }
  .task-time { font-variant-numeric: tabular-nums; }
  .task-state.running { color: var(--blue-11); }
  .task-state.attention { color: var(--amber-11); }
  .task-state.failed { color: var(--red-11); }
  .task-attention { display: grid; place-items: center; color: var(--amber-11); flex: none; }
  .task-progress { margin-top: 8px; color: var(--gray-11); font-size: 12px; line-height: 1.5; }
  .task-progress :global(.markdown-message) { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; max-height: 3em; color: inherit; line-height: inherit; overflow-wrap: anywhere; }
  .task-progress :global(.markdown-message p) { margin: 0; }
  .dismiss-task { position: absolute; right: 4px; top: 2px; display: grid; place-items: center; color: var(--gray-10); width: 16px; height: 20px; padding: 0; border-radius: 4px; opacity: 0; }
  .notch-task:hover .dismiss-task, .notch-task:focus-within .dismiss-task { opacity: 1; }
  button:hover { color: var(--gray-12); background: var(--gray-3); }
  .empty-state { display: grid; justify-items: center; gap: 10px; padding: 28px 0; color: var(--gray-11); font-size: 11px; }
  .empty-state strong { color: var(--gray-12); font-size: 13px; font-weight: 500; }
  .connection-error, .action-notice { margin: 8px 0; font-size: 12px; }
  .action-notice { flex: none; }
  @keyframes pulse { 50% { opacity: .45; } }
  @media (prefers-reduced-motion: reduce) { .status-dot.busy { animation: none; } .stacked .task-content { transition: none; } }
  @container (max-width: 344px) { .provider-icon { margin-top: max(0px, calc((21px - var(--task-icon-size)) / 2)); } .task-topline { flex-wrap: wrap; gap: 4px; } h3 { flex-basis: 100%; } }
</style>
