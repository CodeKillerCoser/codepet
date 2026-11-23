<script lang="ts">
  import { Dialog as DialogPrimitive } from "bits-ui";
  import type { Snippet } from "svelte";
  import { cn, type WithoutChildrenOrChild } from "../utils";
  let {
    ref = $bindable(null), class: className, level = 3, children, ...restProps
  }: WithoutChildrenOrChild<DialogPrimitive.TitleProps> & { children?: Snippet } = $props();
</script>

<DialogPrimitive.Title bind:ref {level} {...restProps}>
  {#snippet child({ props })}
    <svelte:element this={`h${level}`} {...props} data-slot="dialog-title" class={cn("cn-dialog-title", className)}>
      {@render children?.()}
    </svelte:element>
  {/snippet}
</DialogPrimitive.Title>
