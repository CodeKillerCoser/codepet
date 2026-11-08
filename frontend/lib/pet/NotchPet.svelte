<script lang="ts">
  import { Bell, ChevronDown, ExternalLink, PanelTop, Plug, X } from '@lucide/svelte';
  import PetAvatar from './PetAvatar.svelte';
  import MarkdownMessage from '../MarkdownMessage.svelte';
  import { petStatusLabel, type PetTask } from '../petGateway';
  import type { AppSettings, TaskStatus } from '../types';
  import type { PetData } from './model';
  import type { SurfaceView } from './surface';
  import type { petSizing } from './sizing';
  export let sizing: ReturnType<typeof petSizing>;
  import type { PresentationEvent } from './presentation';
  export let data: PetData;
  export let settings: AppSettings | null;
  export let avatarStatus: TaskStatus;
  export let surface: SurfaceView;
  export let expanded: boolean;
  export let actionNotice: string;
  export let dispatch: (event: PresentationEvent) => void;
  export let dragWindow: () => Promise<void>;
  export let toggleSurface: (event: MouseEvent) => void;
  export let openMain: (event: MouseEvent) => void;
  export let dismissActivity: (event: MouseEvent, task: PetTask) => void;
  let header: HTMLButtonElement;
  let failedIcons = new Set<string>();
  $: latest = data.tasks[0];
  const attention = (task: PetTask) => ['waiting-approval', 'waiting-input'].includes(task.status);
  function focusIn(event: FocusEvent) {
    if ((event.target as HTMLElement).matches(':focus-visible')) dispatch({ type: 'focus' });
  }
  function focusOut(event: FocusEvent) {
    if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node)) dispatch({ type: 'blur' });
  }
  function keyDown(event: KeyboardEvent) {
    if (event.key === 'Escape' && expanded) {
      event.preventDefault(); event.stopPropagation();
      header?.focus(); dispatch({ type: 'collapse' });
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="notch-panel dark theme-dark" class:expanded aria-label="顶部任务面板"
  style={`--notch-max-height: ${sizing.expandedHeight}px`}
  on:mouseenter={() => dispatch({ type: 'enter' })} on:mouseleave={() => dispatch({ type: 'leave' })}
  on:focusin={focusIn} on:focusout={focusOut} on:keydown={keyDown}>
  <div class="notch-header">
    <PetAvatar petId={settings?.pet.selectedPetId ?? 'default'} kind={settings?.pet.kind} imagePath={settings?.pet.imagePath}
      portrait width={sizing.portraitWidth} interactive label="桌宠头像" status={avatarStatus} onDrag={dragWindow} dragging={surface.dragging} />
    <button class="notch-summary" bind:this={header} type="button" aria-expanded={expanded} aria-controls="notch-details"
      aria-label={expanded ? '收起任务面板' : '展开任务面板'}
      title={data.error || (latest ? `${petStatusLabel(latest.status)} · ${latest.title}` : '就绪')}
      on:click={() => dispatch({ type: expanded ? 'collapse' : 'enter' })}>
      <span class="status-dot" class:busy={latest?.status === 'running'} class:attention={latest && attention(latest)} class:error={Boolean(data.error) || latest?.status === 'failed'}></span>
      {#if expanded}<span class="latest-status">{data.error ? '连接中断' : latest ? petStatusLabel(latest.status) : '就绪'}</span>{/if}
      <span class="latest-title">{data.error ? '连接中断' : latest?.title || '就绪'}</span>
      {#if data.tasks.length > 1}<span class="task-count" aria-label={`${data.tasks.length} 项任务`}>{data.tasks.length}</span>{/if}
      <ChevronDown size={13} class={expanded ? 'chevron open' : 'chevron'} aria-hidden="true" />
    </button>
    {#if expanded}
      <div class="panel-actions">
        <button type="button" aria-label="切换为悬浮" title="切换为悬浮" on:click={toggleSurface}><PanelTop size={14} /></button>
        <button type="button" aria-label="打开主窗口" title="打开主窗口" on:click={openMain}><ExternalLink size={14} /></button>
      </div>
    {/if}
  </div>
  {#if expanded}
    <div class="notch-details" id="notch-details">
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="notch-task-list" tabindex="0" role="region" aria-label="任务列表">
        {#if data.error}<p class="connection-error" role="status">活动连接暂不可用，正在重试</p>{/if}
        {#each data.tasks as task (task.id)}
          {@const provider = data.providers.find(item => item.providerId === task.providerId)}
          {@const providerName = provider?.displayName || data.sources.find(source => source.id === task.providerId)?.displayName || task.providerId || 'Agent'}
          <article class="notch-task" aria-label={`${task.title}，${petStatusLabel(task.status)}`}>
            <div class="provider-icon" title={providerName}>
              {#if provider?.icon && !failedIcons.has(provider.icon)}
                <img src={provider.icon} alt={providerName} referrerpolicy="no-referrer" on:error={() => { failedIcons = new Set([...failedIcons, provider.icon!]); }} />
              {:else}<Plug size={24} aria-label={providerName} />{/if}
            </div>
            <div class="task-content">
              <div class="task-topline">
                <h3 title={task.title}>{task.title}</h3>
                <span class="task-state" class:running={task.status === 'running'} class:attention={attention(task)} class:failed={task.status === 'failed'}>{petStatusLabel(task.status)}</span>
                <time class="task-time" datetime={new Date(task.updatedAt).toISOString()} title={`最近更新：${new Date(task.updatedAt).toLocaleString()}`} aria-label={`最近更新 ${new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`}>{new Date(task.updatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
                <span class="provider-badge" class:codex={task.providerId?.toLowerCase().includes('codex')} class:attention={attention(task)} title={providerName}>{providerName}</span>
                {#if attention(task)}<span class="task-attention" title={petStatusLabel(task.status)}><Bell size={13} aria-label={petStatusLabel(task.status)} /></span>{/if}
              </div>
              <div class="task-progress"><MarkdownMessage compact message={task.summary?.trim() || '尚未收到回复内容'} /></div>
            </div>
            <button class="dismiss-task" type="button" aria-label={`隐藏 ${task.title}`} title="隐藏任务" on:click={event => dismissActivity(event, task)}><X size={13} /></button>
          </article>
        {:else}
          <div class="empty-state"><strong>暂时没有任务</strong><span>开始编码后，最新进展会显示在这里</span></div>
        {/each}
      </div>
      {#if actionNotice}<p class="action-notice" role="status">{actionNotice}</p>{/if}
    </div>
  {/if}
</section>

<style>
  .notch-panel { position: relative; width: max-content; max-width: 100%; color: var(--color-text-primary); background: var(--color-black); border-radius: 0 0 18px 18px; cursor: default; color-scheme: dark; }
  .notch-panel::before, .notch-panel::after { content: ''; position: absolute; top: 0; width: 16px; height: 16px; background: var(--color-black); pointer-events: none; }
  .notch-panel::before { left: -16px; mask-image: radial-gradient(circle at 0 100%, transparent 15.5px, black 16px); }
  .notch-panel::after { right: -16px; mask-image: radial-gradient(circle at 100% 100%, transparent 15.5px, black 16px); }
  .notch-panel.expanded { width: 100%; border-radius: 0 0 28px 28px; }
  .notch-header { height: var(--notch-header-height); padding: 6px 12px 6px 8px; display: flex; gap: 6px; align-items: center; }
  button { color: inherit; border: 0; background: transparent; }
  button:focus-visible, .notch-task-list:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: -2px; border-radius: 6px; }
  .notch-summary { display: flex; align-items: center; gap: 7px; min-width: 0; flex: 1; text-align: left; padding: 4px 0; font-size: 12px; }
  .status-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--green-9); flex: none; }
  .status-dot.busy { background: var(--blue-9); animation: pulse 1.6s ease-in-out infinite; }
  .status-dot.attention { background: var(--amber-9); }
  .status-dot.error, .connection-error { color: var(--red-11); }
  .status-dot.error { background: var(--red-9); }
  .latest-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
  .expanded .notch-header { width: 100%; padding-left: 16px; padding-right: 16px; }
  .expanded .latest-title { max-width: none; flex: 1; }
  .task-count { color: var(--gray-11); font-size: 11px; font-variant-numeric: tabular-nums; flex: none; }
  .notch-summary :global(.chevron) { color: var(--gray-10); flex: none; }
  .notch-summary :global(.chevron.open) { transform: rotate(180deg); }
  .panel-actions { display: flex; flex: none; gap: 2px; }
  .panel-actions button { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; color: var(--gray-10); }
  .notch-details { display: flex; flex-direction: column; max-height: calc(var(--notch-max-height) - var(--notch-header-height)); padding-bottom: 14px; animation: reveal 140ms ease-out; }
  .notch-task-list { min-height: 0; overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin; scrollbar-color: var(--gray-7) transparent; padding: 10px 24px 2px; }
  .notch-task { position: relative; display: flex; gap: 14px; align-items: flex-start; padding: 8px 0; }
  .provider-icon { width: 28px; height: 32px; flex: none; display: grid; place-items: center; color: var(--gray-11); }
  .provider-icon img { width: 26px; height: 26px; object-fit: contain; border-radius: 6px; }
  .task-content { flex: 1; min-width: 0; }
  .task-topline { display: flex; align-items: center; gap: 8px; min-height: 24px; }
  h3 { flex: 1; min-width: 0; margin: 0; font-size: 14px; font-weight: 600; line-height: 1.5; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .provider-badge { flex: none; max-width: 34%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 4px 9px; border-radius: 16px; font: 600 11px/1.3 var(--font-family-mono, monospace); color: var(--gray-12); background: var(--gray-3); }
  .provider-badge.codex { background: var(--blue-4); color: var(--blue-12); }
  .provider-badge.attention { background: var(--amber-4); color: var(--amber-12); }
  .latest-status, .task-state, .task-time { flex: none; white-space: nowrap; font-size: 10px; color: var(--gray-11); }
  .latest-status { font-size: 11px; }
  .task-time { font-variant-numeric: tabular-nums; }
  .task-state.running { color: var(--blue-11); }
  .task-state.attention { color: var(--amber-11); }
  .task-state.failed { color: var(--red-11); }
  .task-attention { display: grid; place-items: center; color: var(--amber-11); flex: none; }
  .task-progress { margin-top: 1px; color: var(--gray-11); font-size: 12px; line-height: 1.5; }
  .task-progress :global(.markdown-message) { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; max-height: 3em; color: inherit; line-height: inherit; overflow-wrap: anywhere; }
  .task-progress :global(.markdown-message p) { margin: 0; }
  .dismiss-task { position: absolute; right: -20px; top: 13px; display: grid; place-items: center; color: var(--gray-10); width: 20px; height: 20px; padding: 0; border-radius: 5px; opacity: 0; }
  .notch-task:hover .dismiss-task, .notch-task:focus-within .dismiss-task { opacity: 1; }
  button:hover { color: var(--gray-12); background: var(--gray-3); }
  .notch-summary:hover { background: transparent; }
  .empty-state { display: grid; justify-items: center; gap: 10px; padding: 28px 0; color: var(--gray-11); font-size: 11px; }
  .empty-state strong { color: var(--gray-12); font-size: 13px; font-weight: 500; }
  .connection-error, .action-notice { margin: 8px 16px; font-size: 12px; }
  .action-notice { flex: none; }
  @keyframes pulse { 50% { opacity: .45; } }
  @keyframes reveal { from { opacity: 0; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .notch-details, .status-dot.busy { animation: none; } }
  @media (max-width: 380px) { .notch-task-list { padding-left: 12px; padding-right: 20px; } .notch-task { gap: 8px; } .provider-icon { width: 22px; } .provider-icon img { width: 22px; height: 22px; } .task-topline { flex-wrap: wrap; gap: 4px; } h3 { flex-basis: 100%; } .provider-badge { max-width: 80%; padding: 3px 7px; font-size: 10px; } }
</style>
