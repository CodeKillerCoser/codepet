<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';
  import { useSidePanel } from './sidePanel';

  let {
    open,
    title,
    description,
    closeLabel = '关闭侧栏',
    returnFocus,
    onClose,
    children,
  }: {
    open: boolean;
    title: string;
    description: string;
    closeLabel?: string;
    returnFocus?: HTMLElement;
    onClose: () => void;
    children: Snippet;
  } = $props();
  const controller = useSidePanel();
  const owner = Symbol('side-panel-content');
  $effect(() => {
    if (open) controller.show(owner, { title, description, closeLabel, returnFocus, onClose, content: children });
    else controller.release(owner);
  });
  onDestroy(() => controller.release(owner));
</script>
