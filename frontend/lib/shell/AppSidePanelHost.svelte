<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { get } from 'svelte/store';
  import { X } from '@lucide/svelte';
  import * as Dialog from '../ui/dialog/index';
  import { useSidePanel, type SidePanelEntry } from './sidePanel';
  import SidePanelResizeHandle from './SidePanelResizeHandle.svelte';
  import { fly } from 'svelte/transition';

  const controller = useSidePanel();
  const current = controller.current;
  const mode = controller.mode;
  const resizing = controller.resizing;
  const drawerWidth = controller.drawerWidth;
  const headingId = $props.id();
  const panelCardId = `${headingId}-card`;
  let heading: HTMLElement | null = $state(null);
  let presented = $state<SidePanelEntry | null>(null);
  let reducedMotion = $state(false);
  $effect(() => {
    if ($current) presented = $current;
    else if ($mode === 'column' || reducedMotion) presented = null;
  });
  let focusedEntry: { owner: symbol; trigger?: HTMLElement } | undefined;
  const hasForegroundDialog = () => !!document.querySelector('[aria-modal="true"]:not(.app-side-panel):not([data-state="closed"]), dialog[open]');

  // Keep the same surface underneath an open modal. Apply a pending resize after
  // that modal closes, so its editor and place in the focus stack stay intact.
  onMount(() => {
    const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
    const updateMotion = () => { reducedMotion = motion.matches; };
    updateMotion();
    motion.addEventListener('change', updateMotion);
    controller.setViewportWidth(window.innerWidth);
    const resizeViewport = () => controller.setViewportWidth(window.innerWidth);
    window.addEventListener('resize', resizeViewport);
    const media = window.matchMedia('(min-width: 1180px)');
    const observer = new MutationObserver(() => updateMode());
    const updateMode = () => {
      const nextMode = media.matches ? 'column' : 'drawer';
      if (nextMode === get(mode)) { observer.disconnect(); return; }
      if (hasForegroundDialog()) {
        observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['open', 'aria-modal', 'data-state'] });
        return;
      }
      observer.disconnect();
      controller.setMode(nextMode);
    };
    updateMode();
    media.addEventListener('change', updateMode);
    return () => { observer.disconnect(); media.removeEventListener('change', updateMode); motion.removeEventListener('change', updateMotion); window.removeEventListener('resize', resizeViewport); };
  });
  $effect(() => {
    const entry = $current;
    const panelMode = $mode;
    if (!entry) { focusedEntry = undefined; return; }
    if (panelMode === 'drawer') { focusedEntry = undefined; return; }
    if (panelMode === 'column' && (focusedEntry?.owner !== entry.owner || focusedEntry.trigger !== entry.returnFocus)) {
      focusedEntry = { owner: entry.owner, trigger: entry.returnFocus };
      void tick().then(() => { if ($current === entry && !hasForegroundDialog()) heading?.focus({ preventScroll: true }); });
    }
  });
</script>

<svelte:window onkeydown={(event) => {
  if ($mode === 'column' && !$resizing && $current && event.key === 'Escape' && !event.defaultPrevented && !hasForegroundDialog()) {
    event.preventDefault(); controller.dismiss();
  }
}}/>

{#snippet panelContent(entry: SidePanelEntry)}
  <div class="panel-card" id={panelCardId}>
  <header class="panel-header">
    {#if $mode === 'drawer'}
      <Dialog.Title id={headingId} bind:ref={heading} tabindex={-1}>{entry.title}</Dialog.Title>
    {:else}
      <h3 id={headingId} bind:this={heading} tabindex="-1">{entry.title}</h3>
    {/if}
    <button aria-label={entry.closeLabel} onclick={() => controller.dismiss()}><X size={18}/></button>
  </header>
  <div class="panel-body">{@render entry.content()}</div>
  </div>
{/snippet}

{#if $mode === 'column'}
  {#each $current ? [$current] : [] as entry (entry.owner)}
    <aside class="app-side-panel" data-mode="column" aria-labelledby={headingId}
      inert={!$current || $current.owner !== entry.owner}
      transition:fly={{ x: 24, duration: reducedMotion ? 0 : 220 }}>
      <SidePanelResizeHandle controls={panelCardId} />
      {@render panelContent(entry)}
    </aside>
  {/each}
{:else}
  <Dialog.Root open={!!$current} onOpenChange={(open) => {if (!open) controller.dismiss();}}>
    <Dialog.Content class="app-side-panel" overlayClass="history-overlay" showCloseButton={false}
      data-mode="drawer" aria-labelledby={headingId} inert={!$current}
      style={`--app-drawer-width: ${$drawerWidth ? `${$drawerWidth}px` : '50vw'}`}
      escapeKeydownBehavior={$resizing ? 'ignore' : 'close'}
      onanimationend={(event) => { if (event.target === event.currentTarget && !$current) presented = null; }}
      onOpenAutoFocus={(event) => {event.preventDefault(); heading?.focus({preventScroll:true});}}
      onCloseAutoFocus={(event) => event.preventDefault()}>
      <Dialog.Description class="sr-only">{presented?.description ?? ''}</Dialog.Description>
      {#if presented}
        <SidePanelResizeHandle controls={panelCardId} variant="drawer" />
        {@render panelContent(presented)}
      {/if}
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  :global(.app-side-panel) {
    box-sizing: border-box;
    min-width: 0;
    color: var(--app-text);
    font: var(--font-size-sm-plus)/var(--line-height-body) var(--font-family-ui);
    outline: none;
  }
  aside.app-side-panel {
    grid-column: 3;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    min-height: 0;
    position: relative;
    padding-left: 8px;
    overflow: hidden;
  }
  :global(.cn-dialog-content.app-side-panel) {
    inset: 0 0 0 auto;
    transform: none;
    margin: 0;
    width: var(--app-drawer-width,50vw);
    min-width: calc(100vw / 3);
    max-width: 75vw;
    height: 100dvh;
    max-height: 100dvh;
    display: flex;
    flex-direction: column;
    padding: 8px;
    border: 0;
    background: var(--app-bg);
  }
  :global(.app-side-panel[data-mode="drawer"][data-state="open"]) { animation: app-panel-enter 220ms ease both; }
  :global(.app-side-panel[data-mode="drawer"][data-state="closed"]) { animation: app-panel-exit 220ms ease both; pointer-events: none; }
  @keyframes app-panel-enter { from { transform: translateX(100%); } to { transform: translateX(0); } }
  @keyframes app-panel-exit { from { transform: translateX(0); } to { transform: translateX(100%); } }
  .panel-card { display: flex; flex-direction: column; flex: 1; min-height: 0; min-width: 0; overflow: hidden; border: 1px solid var(--color-main-outline); border-radius: 16px; background: var(--app-surface); }
  .panel-header { display: flex; align-items: center; justify-content: space-between; flex: none; gap: 16px; min-height: 52px; padding: 10px var(--app-content-inset,24px); border-bottom: 1px solid var(--color-main-divider); }
  .panel-header :global(h3) { margin: 0; min-width: 0; font: var(--font-section); overflow-wrap: anywhere; }
  .panel-header :global(h3:focus) { outline: none; }
  .panel-header button { display: grid; place-items: center; flex: none; width: 30px; height: 30px; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--app-muted); cursor: pointer; }
  .panel-header button:hover { background: var(--app-surface-soft); }
  .panel-header button:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  .panel-body { flex: 1; padding: 24px var(--app-content-inset,24px); min-height: 0; overflow: auto; overscroll-behavior: contain; }
  @media (prefers-reduced-motion: reduce) {
    :global(.app-side-panel[data-mode="drawer"][data-state]) { animation: none; }
  }
</style>
