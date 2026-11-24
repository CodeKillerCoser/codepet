<script lang="ts">
  import TaskStatus from './TaskStatus.svelte';
  import type {TaskSummary} from './taskLedger';
  export let summary:TaskSummary;
</script>

<span class="run-counts" aria-label="执行状态统计">
  {#if summary.runCounts.inProgress}<span aria-label={`开发中 ${summary.runCounts.inProgress} 次执行`}><TaskStatus status="in_progress" label={false}/>{summary.runCounts.inProgress} 开发中</span>{/if}
  {#if summary.runCounts.readyForDelivery}<span aria-label={`待确认 ${summary.runCounts.readyForDelivery} 次执行`}><TaskStatus status="ready_for_delivery" label={false}/>{summary.runCounts.readyForDelivery} 待确认</span>{/if}
</span>

<style>
 .run-counts{display:flex;align-items:center;flex-wrap:wrap;gap:var(--space-inline) var(--space-related);font:var(--font-caption);color:var(--task-secondary);}
 .run-counts:empty{display:none;}
 .run-counts>span{display:inline-flex;align-items:center;gap:var(--space-inline);white-space:nowrap;}
</style>
