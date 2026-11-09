<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { ChevronDown } from '@lucide/svelte';
  import { initialPresentation, present, type PresentationEvent } from './presentation';
  import type { PetSurfaceController, SurfaceView } from './surface';
  import type { petSizing } from './sizing';
  import { DockPanelMotion } from './dockPanelMotion';
  export let controller: PetSurfaceController | null;
  export let surface: SurfaceView;
  export let pointer: { x: number; y: number } | null = null;
  export let sizing: ReturnType<typeof petSizing>;
  export let title = '';
  export let dragWindow: () => Promise<void>;
  export let failed: (error: unknown) => void;
  let state = { ...initialPresentation, surface };
  let frame: HTMLElement, panel: HTMLElement, summary: HTMLButtonElement;
  let body: HTMLElement, status: HTMLElement, actions: HTMLElement;
  let mounted = false;
  let motion: DockPanelMotion | undefined;
  let openTimer: ReturnType<typeof setTimeout>, closeTimer: ReturnType<typeof setTimeout>;
  let press: { x: number; y: number } | null = null;
  let dragged = false;
  let resize: ResizeObserver | undefined, mutations: MutationObserver | undefined;
  let refreshQueued = false;
  function refresh() {
    if (refreshQueued) return;
    refreshQueued = true;
    void tick().then(() => {
      refreshQueued = false;
      if (!mounted) return;
      frame.style.setProperty('--dock-status-width', `${status.getBoundingClientRect().width}px`);
      frame.style.setProperty('--dock-actions-width', `${actions.getBoundingClientRect().width}px`);
      motion?.refresh();
    });
  }
  function send(event: PresentationEvent) {
    const previous = state;
    state = present(state, event);
    if (state.expanded !== previous.expanded) void motion?.request(state.expanded);
    if (!state.openPending) clearTimeout(openTimer);
    else if (!previous.openPending) openTimer = setTimeout(() => send({ type: 'open-delay' }), 200);
    if (state.pointerHeld || state.surface.dragging || state.hovered || state.focused || !state.expanded) clearTimeout(closeTimer);
    else if (['leave', 'blur', 'release', 'surface'].includes(event.type)) {
      clearTimeout(closeTimer); closeTimer = setTimeout(() => send({ type: 'close-delay' }), 240);
    }
  }
  $: if (surface !== state.surface) { send({ type: 'surface', surface }); refresh(); }
  $: if (surface.dragging) { dragged = true; press = null; }
  $: if (mounted && controller && !motion) { motion = new DockPanelMotion(frame, controller, failed); refresh(); }
  $: if (sizing) refresh();
  $: if (mounted && pointer && !document.hasFocus() && !surface.dragging) {
    const bounds = panel.getBoundingClientRect();
    const inside = pointer.x >= bounds.left && pointer.x < bounds.right && pointer.y >= bounds.top && pointer.y < bounds.bottom;
    hover(inside);
  }
  onMount(() => {
    mounted = true;
    resize = new ResizeObserver(refresh);
    for (const node of [body, status, actions]) resize.observe(node);
    mutations = new MutationObserver(refresh);
    mutations.observe(panel, { subtree: true, childList: true, characterData: true });
    refresh();
  });
  onDestroy(() => {
    mounted = false; clearTimeout(openTimer); clearTimeout(closeTimer);
    resize?.disconnect(); mutations?.disconnect(); motion?.dispose();
  });
  function hover(inside: boolean, pointerMoved = false) {
    // Geometry changes can synthesize enter/leave under a stationary cursor.
    // Real pointer movement still updates input immediately during a morph.
    if (!pointerMoved && frame?.hasAttribute('data-morphing')) return;
    if (inside !== state.hovered) send({ type: inside ? 'enter' : 'leave' });
  }
  function down(event: PointerEvent) {
    if (event.button !== 0) return;
    dragged = false; press = { x: event.clientX, y: event.clientY }; send({ type: 'press' });
  }
  function move(event: PointerEvent) {
    const bounds = panel.getBoundingClientRect();
    hover(event.clientX >= bounds.left && event.clientX < bounds.right && event.clientY >= bounds.top && event.clientY < bounds.bottom, true);
    if (!press || dragged || Math.hypot(event.clientX - press.x, event.clientY - press.y) < 5) return;
    dragged = true; press = null; void dragWindow();
  }
  function release() { press = null; send({ type: 'release' }); }
  function click(event: MouseEvent) {
    if (dragged && event.detail !== 0) { event.preventDefault(); event.stopPropagation(); }
  }
  function key(event: KeyboardEvent) {
    if (event.key === 'Escape' && state.expanded) {
      event.preventDefault(); event.stopPropagation(); summary.focus(); send({ type: 'collapse' });
    }
  }
</script>

<svelte:window on:pointermove={move} on:pointerup={release} on:pointercancel={release} />
<div class="dock-frame" data-dock-frame bind:this={frame}
  style:--dock-collapsed-width={`${sizing.collapsedWidth}px`} style:--dock-expanded-width={`${sizing.expandedWidth}px`}
  style:--dock-safe-inset={`${surface.topInset || 0}px`} style:--notch-min-width={`${surface.topInset ? surface.notchWidth || 0 : 0}px`}
  style:--notch-max-height={`${sizing.expandedHeight}px`} style:--notch-header-height={`${sizing.headerHeight}px`}
  style:--portrait-width={`${sizing.portraitWidth}px`} style:--task-icon-size={`${sizing.portraitWidth * 2 / 3}px`}>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <section class="notch-panel dark theme-dark" class:expanded={state.expanded} bind:this={panel} aria-label="顶部任务面板"
    on:mouseenter={() => hover(true)} on:mouseleave={() => hover(false)}
    on:focusin={event => { if ((event.target as HTMLElement).matches(':focus-visible')) send({ type: 'focus' }); }}
    on:focusout={event => { if (!panel.contains(event.relatedTarget as Node)) send({ type: 'blur' }); }} on:keydown={key}>
    <div class="notch-backdrop" aria-hidden="true"></div>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
    <div class="notch-drag-region" data-pet-drag-region on:pointerdown={down} on:click|capture={click}>
      <div class="notch-header">
        <div class="dock-leading"><slot name="leading" /></div>
        <button class="notch-summary" bind:this={summary} type="button" aria-expanded={state.expanded} aria-controls="notch-details"
          aria-label={state.expanded ? '收起任务面板' : '展开任务面板'} {title} on:click={() => send({ type: state.expanded ? 'collapse' : 'expand' })}>
          <span class="dock-indicator"><slot name="indicator" /></span>
          <span class="dock-status-reveal"><span bind:this={status} class="dock-status"><slot name="status" /></span></span>
          <span class="dock-summary-text"><slot name="summary" /></span>
          <span class="dock-trailing"><slot name="trailing" /></span>
          <ChevronDown size={13} class={state.expanded ? 'chevron open' : 'chevron'} aria-hidden="true" />
        </button>
        <div class="dock-actions-reveal" inert={!state.expanded} aria-hidden={!state.expanded}>
          <div class="panel-actions" bind:this={actions}><slot name="actions" /></div>
        </div>
      </div>
    </div>
    <div class="dock-body-clip" inert={!state.expanded} aria-hidden={!state.expanded}>
      <div class="notch-details" id="notch-details" bind:this={body}><slot /></div>
    </div>
  </section>
</div>

<style>
  .dock-frame { --dock-progress: 0; --dock-inset: var(--dock-safe-inset); position: relative; width: max-content; max-width: var(--dock-collapsed-width); padding: 0 6px; }
  .dock-frame:global([data-ready]) { max-width: none; }
  .notch-panel { position: relative; isolation: isolate; width: 100%; height: 100%; min-width: var(--notch-min-width); padding-top: var(--dock-inset); color: var(--color-text-primary); color-scheme: dark; border-radius: 0 0 calc(8px + 4px * var(--dock-progress)) calc(8px + 4px * var(--dock-progress)); cursor: default; }
  .notch-backdrop { position: absolute; inset: 0; z-index: -1; background: var(--color-black); border-radius: inherit; pointer-events: none; }
  .notch-backdrop::before, .notch-backdrop::after { content: ''; position: absolute; top: 0; width: 6px; height: 6px; background: var(--color-black); pointer-events: none; }
  .notch-backdrop::before { left: -6px; mask-image: radial-gradient(circle at 0 100%, transparent 5.5px, black 6px); }
  .notch-backdrop::after { right: -6px; mask-image: radial-gradient(circle at 100% 100%, transparent 5.5px, black 6px); }
  .notch-drag-region { padding: calc(12px * var(--dock-progress)) calc(6px + 6px * var(--dock-progress)); cursor: grab; }
  .notch-drag-region:active { cursor: grabbing; }
  .notch-header { display: flex; align-items: center; gap: 8px; height: var(--notch-header-height); min-width: 0; }
  .dock-leading { flex: none; display: flex; align-items: center; }
  .notch-summary { display: flex; align-items: center; flex: 1; min-width: 0; padding: 0; border: 0; background: transparent; color: inherit; text-align: left; font-size: 12px; line-height: 1.4; }
  .notch-summary:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: -2px; border-radius: 6px; }
  .dock-indicator { display: flex; flex: none; margin-right: 8px; }
  .dock-status-reveal { width: calc((var(--dock-status-width, 0px) + 8px) * var(--dock-progress)); flex: none; overflow: hidden; opacity: var(--dock-progress); }
  .dock-status { display: block; width: max-content; white-space: nowrap; }
  .dock-summary-text { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .dock-trailing { flex: none; margin-left: 8px; }
  .dock-trailing:empty { display: none; }
  .notch-summary :global(.chevron) { color: var(--gray-10); flex: none; margin-left: 8px; }
  .notch-summary :global(.chevron.open) { transform: rotate(180deg); }
  .dock-actions-reveal { width: calc(var(--dock-actions-width, 0px) * var(--dock-progress)); flex: none; opacity: var(--dock-progress); overflow: hidden; }
  .panel-actions { display: flex; gap: 2px; width: max-content; }
  .dock-body-clip { position: absolute; left: 12px; right: 12px; top: calc(var(--dock-inset) + var(--notch-header-height) + 24px * var(--dock-progress)); bottom: calc(12px * var(--dock-progress)); overflow: hidden; opacity: var(--dock-progress); }
  .notch-details { width: calc(var(--dock-expanded-width) - 36px); display: flex; flex-direction: column; max-height: calc(var(--notch-max-height) - var(--notch-header-height) - var(--dock-inset) - 36px); }
  .dock-frame:global([data-phase="closed"]) .dock-body-clip, .dock-frame:global([data-phase="closed"]) .dock-actions-reveal { visibility: hidden; }
  .dock-frame:global([data-measure]) { height: auto !important; left: 0 !important; }
  .dock-frame:global([data-measure]) .notch-panel { height: auto; }
  .dock-frame:global([data-measure="collapsed"]) { width: max-content !important; max-width: var(--dock-collapsed-width) !important; --dock-progress: 0 !important; }
  .dock-frame:global([data-measure="expanded"]) { width: var(--dock-expanded-width) !important; max-width: none !important; --dock-progress: 1 !important; }
  .dock-frame:global([data-measure="expanded"]) .dock-body-clip { position: relative; top: auto; left: auto; right: auto; bottom: auto; margin: 0 12px; padding-bottom: 12px; }
</style>
