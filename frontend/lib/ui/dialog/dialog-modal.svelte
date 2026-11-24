<script lang="ts">
  import type { Snippet } from 'svelte';
  import Root from './dialog.svelte';
  import Content from './dialog-content.svelte';
  import Title from './dialog-title.svelte';

  // Imperative entry for forms and existing showModal/close callers, backed by
  // the same portal, window stack and focus handling as declarative Dialogs.
  let { title, class: className, children, onCancel }: {
    title: string; class?: string; children: Snippet; onCancel?: (event: KeyboardEvent) => void;
  } = $props();
  let opened = $state(false);
  export function showModal() { opened = true; }
  export function close() { opened = false; }
  export function isOpen() { return opened; }
</script>

<Root bind:open={opened}>
  <Content class={className} showCloseButton={false} interactOutsideBehavior="ignore" onEscapeKeydown={onCancel}>
    <Title class="sr-only">{title}</Title>
    {@render children()}
  </Content>
</Root>
