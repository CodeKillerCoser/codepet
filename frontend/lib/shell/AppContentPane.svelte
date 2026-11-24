<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    heading,
    navigation,
    workbench = false,
    contentElement = $bindable(),
    children,
  }: {
    heading?: Snippet;
    navigation?: Snippet;
    workbench?: boolean;
    contentElement?: HTMLDivElement;
    children: Snippet;
  } = $props();
</script>

<section class="content-pane" class:history-workbench={workbench}>
  {#if heading}<header class="topbar">{@render heading()}</header>{/if}
  {#if navigation}<div class="content-navigation">{@render navigation()}</div>{/if}
  <div class="content" bind:this={contentElement}>{@render children()}</div>
</section>
