<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { get } from 'svelte/store';
  import { X } from '@lucide/svelte';
  import * as Dialog from '../ui/dialog/index';
  import { useSidePanel } from './sidePanel';

  const controller = useSidePanel();
  const current = controller.current;
  const mode = controller.mode;
  const headingId = $props.id();
  let heading: HTMLElement | null = $state(null);
  let focusedEntry: { owner: symbol; trigger?: HTMLElement } | undefined;
  const hasForegroundDialog = () => !!document.querySelector('[aria-modal="true"]:not(.app-side-panel):not([data-state="closed"]), dialog[open]');

  // Keep the same surface underneath an open modal. Apply a pending resize after
  // that modal closes, so its editor and place in the focus stack stay intact.
  onMount(() => {
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
    return () => { observer.disconnect(); media.removeEventListener('change', updateMode); };
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
  if ($mode === 'column' && $current && event.key === 'Escape' && !event.defaultPrevented && !hasForegroundDialog()) {
    event.preventDefault(); controller.dismiss();
  }
}}/>

{#snippet panelContent()}
  {#if $current}
    <header class="panel-header">
      {#if $mode === 'drawer'}
        <Dialog.Title id={headingId} bind:ref={heading} tabindex={-1}>{$current.title}</Dialog.Title>
      {:else}
        <h3 id={headingId} bind:this={heading} tabindex="-1">{$current.title}</h3>
      {/if}
      <button aria-label={$current.closeLabel} onclick={() => controller.dismiss()}><X size={18}/></button>
    </header>
    <div class="panel-body">{@render $current.content()}</div>
  {/if}
{/snippet}

{#if $mode === 'column'}
  {#if $current}
    <aside class="app-side-panel" data-mode="column" aria-labelledby={headingId}>
      {@render panelContent()}
    </aside>
  {/if}
{:else}
  <Dialog.Root open={!!$current} onOpenChange={(open) => {if (!open) controller.dismiss();}}>
    <Dialog.Content class="app-side-panel" overlayClass="history-overlay" showCloseButton={false}
      data-mode="drawer" aria-labelledby={headingId}
      onOpenAutoFocus={(event) => {event.preventDefault(); heading?.focus({preventScroll:true});}}
      onCloseAutoFocus={(event) => event.preventDefault()}>
      <Dialog.Description class="sr-only">{$current?.description ?? ''}</Dialog.Description>
      {@render panelContent()}
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  :global(.app-side-panel) {
    box-sizing: border-box;
    min-width: 0;
    background: var(--app-surface);
    color: var(--app-text);
    font: 13px/1.5 var(--font-family-ui);
    outline: none;
  }
  aside.app-side-panel {
    grid-column: 3;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin-left: 8px;
    border: 1px solid var(--app-border);
    border-radius: 12px;
    overflow: hidden;
  }
  :global(.cn-dialog-content.app-side-panel) {
    inset: 0 0 0 auto;
    transform: none;
    margin: 0;
    width: min(440px,100vw);
    height: 100dvh;
    max-height: 100dvh;
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 0;
    border-left: 1px solid var(--app-border);
    box-shadow: -12px 0 40px var(--color-history-drawer-shadow);
  }
  .panel-header { display: flex; align-items: flex-start; justify-content: space-between; flex: none; gap: 16px; padding: 20px 20px 12px; }
  .panel-header :global(h3) { margin: 0; font-size: 15px; font-weight: 600; line-height: 1.5; overflow-wrap: anywhere; }
  .panel-header :global(h3:focus) { outline: none; }
  .panel-header button { display: grid; place-items: center; flex: none; width: 30px; height: 30px; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--app-muted); cursor: pointer; }
  .panel-header button:hover { background: var(--app-surface-soft); color: var(--app-text); }
  .panel-header button:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  .panel-body { min-height: 0; overflow: auto; overscroll-behavior: contain; }
</style>
