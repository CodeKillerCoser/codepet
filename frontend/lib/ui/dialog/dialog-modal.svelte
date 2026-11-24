<script lang="ts">
  import type { Snippet } from 'svelte';
  import Root from './dialog.svelte';
  import Content from './dialog-content.svelte';
  import Title from './dialog-title.svelte';

  // Imperative entry for forms and existing showModal/close callers, backed by
  // the same portal, window stack and focus handling as declarative Dialogs.
  // A disappearing menu item can return to its persistent menu trigger.
  let { title, class: className, children, onCancel, returnFocus }: {
    title: string; class?: string; children: Snippet; onCancel?: (event: KeyboardEvent) => void; returnFocus?: HTMLElement;
  } = $props();
  let opened = $state(false);
  export function showModal() { opened = true; }
  export function close() { opened = false; }
  export function isOpen() { return opened; }
</script>

<Root bind:open={opened}>
  <Content class={className} showCloseButton={false} interactOutsideBehavior="ignore" onEscapeKeydown={onCancel}
    onCloseAutoFocus={event => { if (returnFocus?.isConnected) { event.preventDefault(); returnFocus.focus({ preventScroll: true }); } }}>
    <Title class="sr-only">{title}</Title>
    {@render children()}
  </Content>
</Root>
