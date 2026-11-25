<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    heading,
    navigation,
    workbench = false,
    canvas = false,
    contentElement = $bindable(),
    children,
  }: {
    heading?: Snippet;
    navigation?: Snippet;
    workbench?: boolean;
    canvas?: boolean;
    contentElement?: HTMLDivElement;
    children: Snippet;
  } = $props();
</script>

<section class="content-pane" class:history-workbench={workbench} class:canvas>
  {#if heading}<header class="topbar">{@render heading()}</header>{/if}
  {#if navigation}<div class="content-navigation">{@render navigation()}</div>{/if}
  <div class="content" bind:this={contentElement}>{@render children()}</div>
</section>
