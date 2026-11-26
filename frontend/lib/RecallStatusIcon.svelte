<script lang="ts">
  import { Check, X, Hourglass, LoaderCircle, Ban, Pause, RotateCcw } from '@lucide/svelte';
  import { processingLabels, type ProcessingState } from './recallHistory';
  export let state: ProcessingState;
  export let size = 15;
</script>
<span class="processing-state" class:success={state === 'completed'} class:failure={state === 'failed'} class:working={state === 'processing'} role="img" aria-label={processingLabels[state]} title={processingLabels[state]}>
  {#if state === 'completed'}<Check {size}/>{:else if state === 'failed'}<X {size}/>{:else if state === 'processing'}<LoaderCircle {size}/>{:else if state === 'queued'}<Hourglass {size}/>{:else if state === 'paused'}<Pause {size}/>{:else if state === 'cancelled'}<Ban {size}/>{:else}<RotateCcw {size}/>{/if}
</span>
<style>
  .processing-state{display:inline-flex;align-items:center;flex:none;color:var(--app-muted)}.success{color:var(--app-good)}.failure{color:var(--app-danger)}.working{color:var(--app-accent)}.working :global(svg){animation:processing-spin 1.2s linear infinite}@keyframes processing-spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){.working :global(svg){animation:none}}
</style>
