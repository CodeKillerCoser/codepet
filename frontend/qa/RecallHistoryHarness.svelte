<script lang="ts">
  import { provideSidePanel } from '../lib/shell/sidePanel';
  import AppSidePanelHost from '../lib/shell/AppSidePanelHost.svelte';
  import RecallHistory from '../lib/RecallHistory.svelte';
  const panel = provideSidePanel();
  const current = panel.current;
  let open = false;
  let trigger: HTMLButtonElement;
</script>
<main class="main-theme" data-side-panel-layout>
  <button bind:this={trigger} onclick={() => open = !open}>加工历史</button>
  <div>图谱画布</div>
  <AppSidePanelHost/>
  <RecallHistory {open} returnFocus={trigger} onClose={() => open = false}/>
  <span hidden>{!!$current}</span>
</main>
