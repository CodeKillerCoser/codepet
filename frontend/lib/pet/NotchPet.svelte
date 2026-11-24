<script lang="ts">
  import NoticeActions from '../NoticeActions.svelte';
  import { noticeStatusLabel, noticeNeedsInput } from '../noticePresentation';

  import { onMount, onDestroy } from 'svelte';
  import { PlaneStackMotion, planeStackCards, type PlaneStackFrame } from './planeStackMotion';
  import { Bell, ExternalLink, PanelTop, Pin, Plug, X } from '@lucide/svelte';
  import PetAvatar from './PetAvatar.svelte';
  import MarkdownMessage from '../MarkdownMessage.svelte';
  import WindowDock from './WindowDock.svelte';
  import { defaultDrawerSize, drawerCommand, type DrawerWindow, type DrawerPointer, type DrawerRect } from './windowDrawer';
  import DockPanel from './DockPanel.svelte';
  import type { PetSurfaceController } from './surface';
  import { type Notice } from '../noticeGateway';
  import type { AppSettings, TaskStatus } from '../types';
  import type { PetData } from './model';
  import type { SurfaceView } from './surface';
  import type { petSizing } from './sizing';
  export let syncHitRegions: () => Promise<void>;
  export let sizing: ReturnType<typeof petSizing>;
  export let data: PetData;
  export let settings: AppSettings | null;
  export let avatarStatus: TaskStatus;
  export let surface: SurfaceView;
  export let controller: PetSurfaceController | null;
  export let pointer: DrawerPointer | null;
  export let failed: (error: unknown) => void;
  export let actionNotice: string;
  export let dragWindow: () => Promise<void>;
  export let toggleSurface: (event: MouseEvent) => void;
  export let openMain: (event: MouseEvent) => void;
  export let dismissActivity: (event: MouseEvent, task: Notice) => void;
  let dropReady = false;
  let drawerSupported = false, helpOpen = false, pinned = false;
  let resizing = false;
  let visibleRects: DrawerRect[] = [];
  let selectedWindow: DrawerWindow | null = null;
  let dock: WindowDock;
  let windowHost: HTMLElement | undefined;
  let panel: DockPanel;
  // Prefer 800 × 560; expand only to dimensions the native app actually enforces.
  let notificationHeight = 0;
  $: drawerChromeHeight = sizing.headerHeight + (surface.topInset || 0) + 36 + Math.min(85, notificationHeight);
  $: maxWidth = Math.max(1, (surface.availableWidth ?? 836) - 36);
  $: maxHeight = Math.max(1, surface.availableHeight - drawerChromeHeight);
  $: hostHeight = Math.min(selectedWindow?.requiredHeight ?? defaultDrawerSize.height, maxHeight);
  $: panelSizing = selectedWindow ? {
    ...sizing,
    expandedWidth: Math.min(selectedWindow.requiredWidth ?? defaultDrawerSize.width, maxWidth) + 36,
    expandedHeight: Math.min(surface.availableHeight, hostHeight + drawerChromeHeight),
  } : sizing;
  onMount(() => {
    let mounted = true;
    void drawerCommand('supported').then(value => { if (mounted) drawerSupported = value === true; }).catch(() => {});
    return () => { mounted = false; };
  });
  let failedIcons = new Set<string>();
  let expandedCompleted = new Set<string>();
  const stackMotions = new Map<string, PlaneStackMotion>();
  let stackFrames = new Map<string, PlaneStackFrame>();
  let reducedMotion = false;
  onMount(() => {
    const preference = matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => { reducedMotion = preference.matches; };
    update(); preference.addEventListener('change', update);
    return () => preference.removeEventListener('change', update);
  });
  onDestroy(() => { for (const motion of stackMotions.values()) motion.destroy(); });
  const wheelProgress = new Map<string, { delta: number; at: number; flippedAt: number }>();

  const attention = (task: Notice) => noticeNeedsInput(task);
  const live = (task: Notice) => !task.finished || attention(task);
  const completedOrCancelled = (task: Notice) => task.finished;
  function completedByProvider(tasks: Notice[]) {
    const groups = new Map<string, { id: string; providerId: string; completed: boolean; tasks: Notice[] }>();
    for (const task of tasks) {
      if (!completedOrCancelled(task)) continue;
      const providerId = task.pluginId || '';
      const id = `completed-${encodeURIComponent(providerId)}`;
      if (!groups.has(id)) groups.set(id, { id, providerId, completed: true, tasks: [] });
      groups.get(id)!.tasks.push(task);
    }
    return [...groups.values()];
  }
  $: completedGroups = completedByProvider(data.notices);
  $: taskGroups = [
    { id: 'active', providerId: '', completed: false, tasks: data.notices.filter(task => !completedOrCancelled(task)).sort((a, b) => Number(live(b)) - Number(live(a))) },
    ...completedGroups,
  ].filter(group => group.tasks.length > 0);
  $: if ([...expandedCompleted].some(id => !completedGroups.some(group => group.id === id))) {
    expandedCompleted = new Set([...expandedCompleted].filter(id => completedGroups.some(group => group.id === id)));
  }
  $: syncStacks(completedGroups, expandedCompleted, reducedMotion);
  function syncStacks(groups: ReturnType<typeof completedByProvider>, expanded: Set<string>, reduced: boolean) {
    for (const [id, motion] of stackMotions) {
      if (!groups.some(group => group.id === id)) {
        motion.destroy(); stackMotions.delete(id); stackFrames.delete(id); wheelProgress.delete(id);
      }
    }
    for (const group of groups) {
      let motion = stackMotions.get(group.id);
      if (!motion) {
        motion = new PlaneStackMotion(frame => { stackFrames = new Map(stackFrames).set(group.id, frame); });
        stackMotions.set(group.id, motion);
      }
      motion.sync(group.tasks.map(task => task.noticeId), !expanded.has(group.id), reduced);
    }
  }
  function rotateStack(event: WheelEvent, id: string) {
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
    stackMotions.get(id)?.step(Math.sign(accumulated));
    wheelProgress.set(id, { delta: 0, at: now, flippedAt: now });
  }
  function toggleCompleted(id: string) {
    wheelProgress.delete(id);
    const next = new Set(expandedCompleted);
    if (next.has(id)) next.delete(id); else next.add(id);
    expandedCompleted = next;
  }
  $: latest = data.notices[0];
  function expandCompleted(event: MouseEvent, id: string) {
    if (!expandedCompleted.has(id) && !(event.target as Element).closest('button, a, .notice-actions')) toggleCompleted(id);
  }
</script>

<DockPanel bind:this={panel} {controller} {surface} {pointer} {resizing} sizing={panelSizing} {dragWindow} {failed}
  holdOpen={pinned || helpOpen} beforeCollapse={selectedWindow ? () => dock.beforeCollapse() : undefined}
  prepareWindow={selectedWindow ? () => dock.prepareReveal() : undefined}
  onExpand={() => dock?.resumed()}
  title={data.error || (latest ? `${noticeStatusLabel(latest)} · ${latest.content.title}` : '就绪')}>
  <svelte:fragment slot="leading">
    <PetAvatar petId={settings?.pet.selectedPetId ?? 'default'} kind={settings?.pet.kind} imagePath={settings?.pet.imagePath}
      portrait width={sizing.portraitWidth} interactive label="桌宠头像" status={avatarStatus} dragging={surface.dragging} />
  </svelte:fragment>
  <span slot="indicator" class="status-dot" class:busy={latest && !latest.finished} class:attention={latest && attention(latest)} class:error={Boolean(data.error)}></span>
  <span slot="status" class="latest-status">{data.error ? '连接中断' : latest ? noticeStatusLabel(latest) : '就绪'}</span>
  <span slot="summary" class="latest-title">{data.error ? '连接中断' : latest?.content.title || '就绪'}</span>
  <svelte:fragment slot="trailing">
    {#if data.notices.length > 1}<span class="task-count" aria-label={`${data.notices.length} 条通知`}>{data.notices.length}</span>{/if}
  </svelte:fragment>
  <svelte:fragment slot="actions">
    {#if drawerSupported}<button class="panel-action" type="button" aria-pressed={pinned} aria-label={pinned ? '取消固定' : '固定展开'} title={pinned ? '取消固定' : '固定展开'} on:click={() => pinned = !pinned}><Pin size={14}/></button>{/if}
    <button class="panel-action" type="button" aria-label="切换为悬浮" title="切换为悬浮" on:click={toggleSurface}><PanelTop size={14} /></button>
    <button class="panel-action" type="button" aria-label="打开主窗口" title="打开主窗口" on:click={openMain}><ExternalLink size={14} /></button>
  </svelte:fragment>
  <svelte:fragment slot="dragHint">
    {#if dropReady}<div class="drop-hint" role="status">松手钉住窗口</div>{/if}
  </svelte:fragment>
  <svelte:fragment slot="overlay" let:frame let:panel={shell} let:backdrop let:body>
    {#if drawerSupported}<WindowDock {syncHitRegions} {frame} panel={shell} {backdrop} {body} host={windowHost} bind:this={dock} bind:dropReady bind:selected={selectedWindow} bind:helpOpen bind:visibleRects bind:resizing {maxWidth} {maxHeight} {failed} {pointer} {pinned} openPanel={() => panel.expandForWindow()}/>{/if}
  </svelte:fragment>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="notch-task-list" bind:clientHeight={notificationHeight} class:hosting={selectedWindow} class:help-open={helpOpen} tabindex="0" role="region" aria-label="任务列表">
        {#if data.error}<p class="connection-error" role="status">活动连接暂不可用，正在重试</p>{/if}
        {#each taskGroups as group (group.id)}
          {@const completedExpanded = expandedCompleted.has(group.id)}
          {@const stacked = group.completed && !completedExpanded && group.tasks.length > 1}
          {@const frame = stackFrames.get(group.id)}
          {@const cards = stacked ? planeStackCards(group.tasks, frame?.position ?? 0) : group.tasks.map(item => ({ item, front: true, y: 0, scale: 1, opacity: 1, ink: 1, layer: 0, depth: 0 }))}
          {#if group.completed && group.id === completedGroups[0]?.id}
            <h2 class="completed-heading">已结束 <span class="completed-count">{completedGroups.reduce((count, item) => count + item.tasks.length, 0)}</span></h2>
          {/if}
          <!-- The first provider icon toggles its group with pointer or keyboard. -->
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
          <div class="task-group" class:stacked class:completed={group.completed} role="group" aria-label={group.completed ? `${group.providerId || 'Agent'} 已结束条目` : '当前任务'}
            id={`notch-${group.id}`}
            style={`--stack-count: ${Math.min(group.tasks.length - 1, 2)}`}
            data-plane-moving={stacked && frame?.moving ? '' : undefined}
            data-plane-position={stacked ? frame?.position ?? 0 : undefined}
            on:click={event => { if (group.completed) expandCompleted(event, group.id); }}
            on:wheel|nonpassive={event => { if (stacked) rotateStack(event, group.id); }}>
            {#if group.completed}
              {@const provider = data.providers.find(item => item.providerId === group.providerId)}
              {@const providerName = provider?.displayName || data.sources.find(source => source.pluginId === group.providerId)?.displayName || group.providerId || 'Agent'}
              <div class="provider-icon group-icon" title={providerName}>
                <button class="completed-toggle" type="button" aria-expanded={completedExpanded}
                  aria-label={`${providerName} 已结束条目（${group.tasks.length}）`}
                  title={`${completedExpanded ? '收起' : '展开'} ${providerName} 的已结束条目`}
                  aria-controls={`notch-${group.id}-tasks`} on:click={() => toggleCompleted(group.id)}>
                  {#if provider?.icon && !failedIcons.has(provider.icon)}
                    <img src={provider.icon} alt={providerName} referrerpolicy="no-referrer" on:error={() => { failedIcons = new Set([...failedIcons, provider.icon!]); }} />
                  {:else}<Plug size={sizing.portraitWidth * 2 / 3} aria-label={providerName} />{/if}
                </button>
              </div>
            {/if}
            <div class="task-group-items" id={`notch-${group.id}-tasks`}>
              {#each cards as card (card.item.noticeId)}
                {@const task = card.item}
                {@const provider = data.providers.find(item => item.providerId === task.pluginId)}
                {@const providerName = provider?.displayName || data.sources.find(source => source.pluginId === task.pluginId)?.displayName || task.pluginId || 'Agent'}
                <article class="notch-task" class:actionable={task.actions.some(action => action.enabled && ['reply','permission','activate'].includes(action.kind))} class:stack-back={stacked && !card.front}
                  inert={stacked && !card.front} aria-hidden={stacked && !card.front ? 'true' : undefined}
                  data-task-id={task.noticeId}
                  style={stacked ? `--plane-y: ${card.y}px; --plane-scale: ${card.scale}; --plane-ink: ${card.ink}; --plane-depth: ${card.depth}; opacity: ${card.opacity}; z-index: ${card.layer}` : undefined}
                  aria-label={`${task.content.title}，${noticeStatusLabel(task)}`}>
                  {#if !group.completed}
                    <div class="provider-icon" title={providerName}>
                      {#if provider?.icon && !failedIcons.has(provider.icon)}
                        <img src={provider.icon} alt={providerName} referrerpolicy="no-referrer" on:error={() => { failedIcons = new Set([...failedIcons, provider.icon!]); }} />
                      {:else}<Plug size={sizing.portraitWidth * 2 / 3} aria-label={providerName} />{/if}
                    </div>
                  {/if}
                  <div class="task-content">
                    <div class="task-topline">
                      <h3 title={task.content.title}>{task.content.title}</h3>
                      <span class="task-state" class:running={!task.finished} class:attention={attention(task)}>{noticeStatusLabel(task)}</span>
                      <time class="task-time" datetime={new Date(task.updatedAt).toISOString()} title={`最近更新：${new Date(task.updatedAt).toLocaleString()}`} aria-label={`最近更新 ${new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`}>{new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
                      {#if attention(task)}<span class="task-attention" title={noticeStatusLabel(task)}><Bell size={13} aria-label={noticeStatusLabel(task)} /></span>{/if}
                    </div>
                    <div class="task-progress"><MarkdownMessage compact message={task.content.progress?.text?.trim() || '暂无进度说明'} /></div>
                    {#if task.content.progress?.percent !== undefined}<progress max="100" value={task.content.progress.percent} aria-label="通知进度"></progress>{/if}
                    <NoticeActions notice={task} available={data.sources.some(source => source.pluginId === task.pluginId && source.enabled && source.connected)} />
                    <button class="dismiss-task" type="button" aria-label={`隐藏 ${task.content.title}`} title="隐藏条目" on:click={event => dismissActivity(event, task)}><X size={13} /></button>
                  </div>
                </article>
              {/each}
            </div>
          </div>
        {:else}
          <div class="empty-state"><strong>暂时没有通知</strong><span>收到来源消息后，最新进展会显示在这里</span></div>
        {/each}
      </div>
      {#if selectedWindow}
        <div bind:this={windowHost} class="window-host" data-window-host aria-label={selectedWindow.name + ' · ' + selectedWindow.title}
          style:height={`${hostHeight}px`}>
          {#each visibleRects as area}
            <div class="native-window-region" data-native-window-region
              style:left={`${area.x}px`} style:top={`${area.y}px`}
              style:width={`${area.width}px`} style:height={`${area.height}px`}></div>
          {/each}
          <button type="button" class="activate-window" data-window-control on:click={() => dock.activateSelected()}>激活{selectedWindow.name}窗口</button>
          <button type="button" class="retry-window" data-window-control on:click={() => dock.activateSelected()}>重新显示{selectedWindow.name}窗口</button>
        </div>
      {/if}
      {#if actionNotice}<p class="action-notice" role="status">{actionNotice}</p>{/if}
</DockPanel>

<style>
  .drop-hint {position:absolute;inset:0;z-index:5;display:grid;place-items:center;background:var(--gray-3);border:1px dashed var(--gray-8);border-radius:8px;color:var(--gray-12);font-size:var(--font-size-sm);pointer-events:none}
  .native-window-region { position:absolute; }
  .window-host { position:relative; min-height:0; flex:none; display:grid; place-items:center; }
  .activate-window, .retry-window { color:var(--gray-11); font:inherit; cursor:pointer; }
  .window-host:not(:global([data-covered])) .activate-window { display:none; }
  .window-host:not(:global([data-missing])) .retry-window { display:none; }
  .notch-task-list.hosting { flex-shrink:0; max-height:85px; }
  .hosting .completed-heading,.hosting .task-group.completed,.hosting .notch-task:not(:first-child),.hosting .empty-state { display:none; }
  .notch-task-list.help-open:not(.hosting) { min-height:min(170px,calc(var(--notch-max-height) - 80px)); }
  .panel-action[aria-pressed="true"] { color:var(--color-text-primary); background:var(--gray-4); }

  button { color: inherit; border: 0; background: transparent; }
  button:focus-visible, .notch-task-list:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: -2px; border-radius: 6px; }
  .panel-action { display: grid; place-items: center; flex: none; width: 22px; height: 22px; border-radius: 6px; color: var(--gray-10); }
  progress { width:100%; height:4px; accent-color:var(--app-accent); }
  .status-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--green-9); flex: none; }
  .status-dot.busy { background: var(--blue-9); animation: pulse 1.6s ease-in-out infinite; }
  .status-dot.attention { background: var(--amber-9); }
  .status-dot.error, .connection-error { color: var(--red-11); }
  .status-dot.error { background: var(--red-9); }
  .latest-title { display: block; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--font-weight-medium); }
  .task-count { color: var(--gray-11); font-size: var(--font-size-xs); font-variant-numeric: tabular-nums; flex: none; }
  .notch-task-list { container-type: inline-size; min-height: 0; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin; scrollbar-color: var(--gray-7) transparent; padding: 0; }
  .task-group { min-width: 0; }
  .task-group.completed { display: grid; grid-template-columns: var(--task-icon-size) minmax(0, 1fr); gap: 2px; align-items: start; }
  .completed .group-icon { position: relative; top: 6px; }
  .task-group-items { min-width: 0; display: grid; gap: 12px; }
  .completed .notch-task { display: flex; flex-direction: column; align-items: stretch; }
  .notch-task { position: relative; min-width: 0; display: grid; grid-template-columns: var(--task-icon-size) minmax(0, 1fr); gap: 8px; align-items: start; }
  .completed-heading { margin: 12px 0 8px; color: var(--gray-11); font-size: var(--font-size-xs); font-weight: var(--font-weight-regular); line-height: var(--line-height-caption-fixed); }
  .completed-count { margin-left: 4px; color: var(--gray-10); font-variant-numeric: tabular-nums; }
  .completed-toggle { display: grid; place-items: center; width: 100%; height: 100%; padding: 0; border-radius: 4px; cursor: pointer; }
  .completed + .completed { margin-top: 12px; }
  .task-group.stacked { cursor: pointer; }
  .stacked .task-group-items { grid-template-columns: minmax(0, 1fr); gap: 0; isolation: isolate; padding: 0 0 calc(var(--stack-count) * 11px + 1px); }
  .stacked .notch-task { grid-area: 1 / 1; }
  .stacked .stack-back { pointer-events: none; }
  .completed .task-content::before { content: ''; position: absolute; inset: 0; z-index: -1; background: color-mix(in srgb, var(--color-black) 20%, var(--gray-2)); border-radius: 6px; }
  .stacked .task-content::before { background: color-mix(in srgb, var(--gray-3) calc(var(--plane-depth) * 30%), var(--gray-2)); box-shadow: inset 0 0 0 1px var(--gray-4); }
  .completed .task-content { flex: 1; isolation: isolate; padding: 6px; }
  .completed .task-topline { padding-right: 0; }
  .completed .notch-task:hover .task-topline, .completed .notch-task:focus-within .task-topline { padding-right: 18px; }
  .completed .dismiss-task { top: 8px; right: 6px; }
  .stacked .task-content { flex: none; box-sizing: border-box; height: 80px; transform: translateY(var(--plane-y)) scale(var(--plane-scale)); transform-origin: center top; }
  .stacked .actionable .task-content { height: auto; min-height: 80px; }
  .stacked .task-topline { padding-right: 18px; }
  .stacked .task-content > * { opacity: var(--plane-ink); }
  .stacked .task-content > .dismiss-task { opacity: 0; }
  .stacked .notch-task:not(.stack-back):hover .dismiss-task, .stacked .notch-task:focus-within .dismiss-task { opacity: var(--plane-ink); }
  [data-plane-moving] .dismiss-task { visibility: hidden; }
  .provider-icon { margin-top: max(0px, calc((24px - var(--task-icon-size)) / 2)); width: var(--task-icon-size); height: var(--task-icon-size); display: grid; place-items: center; color: var(--gray-11); }
  .provider-icon img { width: var(--task-icon-size); height: var(--task-icon-size); object-fit: contain; border-radius: 4px; }
  .task-content { position: relative; min-width: 0; padding: 0 4px 4px 0; }
  .task-topline { display: flex; align-items: center; gap: 8px; min-height: 24px; padding-right: 18px; }
  h3 { flex: 1; min-width: 0; margin: 0; font-size: var(--font-size-label); font-weight: var(--font-weight-semibold); line-height: var(--line-height-loose); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .latest-status, .task-state, .task-time { flex: none; white-space: nowrap; font-size: var(--font-size-2xs); color: var(--gray-11); }
  .latest-status { font-size: var(--font-size-xs); }
  .task-time { font-variant-numeric: tabular-nums; }
  .task-state.running { color: var(--blue-11); }
  .task-state.attention { color: var(--amber-11); }
  .task-state.failed { color: var(--red-11); }
  .task-attention { display: grid; place-items: center; color: var(--amber-11); flex: none; }
  .task-progress { margin-top: 8px; color: var(--gray-11); font-size: var(--font-size-sm); line-height: var(--line-height-loose); }
  .task-progress :global(.markdown-message) { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; max-height: 3em; color: inherit; line-height: inherit; overflow-wrap: anywhere; }
  .task-progress :global(.markdown-message p) { margin: 0; }
  .dismiss-task { position: absolute; right: 4px; top: 2px; display: grid; place-items: center; color: var(--gray-10); width: 16px; height: 20px; padding: 0; border-radius: 4px; opacity: 0; }
  .notch-task:hover .dismiss-task, .notch-task:focus-within .dismiss-task { opacity: 1; }
  button:hover { color: var(--gray-12); background: var(--gray-3); }
  .empty-state { display: grid; justify-items: center; gap: 10px; padding: 28px 0; color: var(--gray-11); font-size: var(--font-size-xs); }
  .empty-state strong { color: var(--gray-12); font-size: var(--font-size-caption); font-weight: var(--font-weight-medium); }
  .connection-error, .action-notice { margin: 8px 0; font-size: var(--font-size-sm); }
  .action-notice { flex: none; }
  @keyframes pulse { 50% { opacity: .45; } }
  @media (prefers-reduced-motion: reduce) { .status-dot.busy { animation: none; } }
  @container (max-width: 344px) { .stacked .task-content { height: 104px; } .provider-icon { margin-top: max(0px, calc((21px - var(--task-icon-size)) / 2)); } .task-topline { flex-wrap: wrap; gap: 4px; } h3 { flex-basis: 100%; } }
</style>
