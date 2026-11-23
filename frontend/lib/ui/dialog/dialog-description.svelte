<script lang="ts">
  import { Dialog as DialogPrimitive } from "bits-ui";
  import type { Snippet } from "svelte";
  import { cn, type WithoutChildrenOrChild } from "../utils";
  let {
    ref = $bindable(null), class: className, children, ...restProps
  }: WithoutChildrenOrChild<DialogPrimitive.DescriptionProps> & { children?: Snippet } = $props();
</script>

<DialogPrimitive.Description bind:ref {...restProps}>
  {#snippet child({ props })}
    <p {...props} data-slot="dialog-description" class={cn("cn-dialog-description", className)}>{@render children?.()}</p>
  {/snippet}
</DialogPrimitive.Description>
