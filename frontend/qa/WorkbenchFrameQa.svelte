<script lang="ts">
  import CodexWorkbench from '../lib/CodexWorkbench.svelte';
  import AppSidePanelHost from '../lib/shell/AppSidePanelHost.svelte';
  import { provideSidePanel } from '../lib/shell/sidePanel';

  // Isolated page QA still uses the production framework controller and host.
  const panel = provideSidePanel();
  const current = panel.current;
  const mode = panel.mode;
</script>

<main class="qa-frame" class:with-panel={!!$current && $mode === 'column'}>
  <div class="frame-page"><CodexWorkbench /></div>
  <AppSidePanelHost />
</main>

<style>
  .qa-frame { display: grid; grid-template-columns: minmax(0,1fr); grid-template-rows: 0 minmax(0,1fr); height: 100dvh; overflow: hidden; }
  .qa-frame.with-panel { grid-template-columns: minmax(0,1fr) 400px; }
  .frame-page { grid-column: 1 / -1; grid-row: 2; min-width: 0; min-height: 0; overflow: auto; }
  .with-panel .frame-page { grid-column: 1; }
  .qa-frame :global(aside.app-side-panel) { grid-column: 2; }
</style>
