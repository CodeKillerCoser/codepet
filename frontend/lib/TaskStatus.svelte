<script lang="ts">
  import {CircleDot, CircleDashed, CircleCheck, Send} from '@lucide/svelte';
  import {statuses,type RunStatus} from './taskLedger';
  export let status:RunStatus;
  export let label=true;
</script>
<span class="task-status" data-status={status} aria-label={label?undefined:statuses[status]}>
  {#if status==='in_progress'}<CircleDot size={14}/>{:else if status==='ready_for_delivery'}<CircleDashed size={14}/>{:else if status==='published'}<Send size={14}/>{:else}<CircleCheck size={14}/>{/if}
  {#if label}<span>{statuses[status]}</span>{/if}
</span>
<style>
 .task-status{display:inline-flex;align-items:center;gap:7px;font-size:var(--font-size-label);white-space:nowrap;color:var(--wb-text);}
 .task-status :global(svg){flex:none;color:var(--task-secondary);}
 .task-status[data-status="in_progress"] :global(svg){color:var(--wb-info-text);}
 .task-status[data-status="ready_for_delivery"] :global(svg){color:var(--wb-warning-strong-text);}
 .task-status[data-status="published"] :global(svg){color:var(--color-success);}
</style>
