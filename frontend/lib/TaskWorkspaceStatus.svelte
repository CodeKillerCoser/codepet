<script lang="ts">
  import {FilePenLine,CloudUpload,CloudCheck,CircleHelp} from '@lucide/svelte';
  import {workspaceCommitState} from './taskChanges';
  import type {AgentWorkspace} from './taskLedger';
  export let workspace:AgentWorkspace;
  $: state=workspaceCommitState(workspace);
</script>
<span class="workspace-status" data-state={state.state} title="基于记录时的工作区和本地上游引用">
  {#if state.state==='dirty'}<FilePenLine size={14}/>{:else if state.state==='unpushed'}<CloudUpload size={14}/>{:else if state.state==='synced'}<CloudCheck size={14}/>{:else}<CircleHelp size={14}/>{/if}
  <span>{state.label}</span>
</span>
<style>
 .workspace-status{display:inline-flex;align-items:center;gap:6px;font-size:var(--font-size-caption);color:var(--task-secondary);}
 .workspace-status :global(svg){flex:none;}
 .workspace-status[data-state="dirty"] :global(svg){color:var(--wb-warning-strong-text);}
 .workspace-status[data-state="unpushed"] :global(svg){color:var(--wb-info-text);}
 .workspace-status[data-state="synced"] :global(svg){color:var(--color-success);}
</style>
