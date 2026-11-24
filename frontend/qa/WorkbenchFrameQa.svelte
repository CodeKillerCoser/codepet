<script lang="ts">
  import CodexWorkbench from '../lib/CodexWorkbench.svelte';
  import AppSidePanelHost from '../lib/shell/AppSidePanelHost.svelte';
  import { provideSidePanel } from '../lib/shell/sidePanel';
  import AppContentPane from '../lib/shell/AppContentPane.svelte';
  import WorkbenchNavigation from '../lib/WorkbenchNavigation.svelte';

  export let view: 'code' | 'tasks' = 'code';

  // Isolated page QA still uses the production framework controller and host.
  const panel = provideSidePanel();
  const current = panel.current;
  const mode = panel.mode;
  const width = panel.width;
  const resizing = panel.resizing;
  let visible = true;
  export function setVisible(value: boolean) { visible = value; }
</script>

{#snippet viewNavigation()}<WorkbenchNavigation bind:view />{/snippet}
<main data-side-panel-layout style:--app-side-panel-width={`${$width}px`} class:resizing-side-panel={$resizing} class="qa-frame" class:with-panel={!!$current && $mode === 'column'}>
  <AppContentPane workbench navigation={visible ? viewNavigation : undefined}>
    {#if visible}<div id="codex-workbench-panel" role="tabpanel" aria-labelledby={`tab-${view}`} tabindex="0"><CodexWorkbench bind:view /></div>{/if}
  </AppContentPane>
  <AppSidePanelHost />
</main>

<style>
  .qa-frame { display: grid; grid-template-columns: minmax(0,1fr) 0px; grid-template-rows: 0 minmax(0,1fr); height: 100dvh; overflow: hidden; transition: grid-template-columns 220ms ease; }
  .qa-frame.with-panel { grid-template-columns: minmax(0,1fr) var(--app-side-panel-width,400px); }
  .qa-frame.resizing-side-panel { transition: none; user-select: none; cursor: col-resize; }
  .qa-frame :global(.content-pane) { grid-column: 1; grid-row: 2; }
  .qa-frame :global(aside.app-side-panel) { grid-column: 2; }
  @media (max-width: 1179px) {
    .qa-frame, .qa-frame.with-panel { grid-template-columns: minmax(0,1fr) 0px; transition: none; }
  }
  @media (prefers-reduced-motion: reduce) { .qa-frame { transition: none; } }
</style>
