<script lang="ts">
	import { Dialog as DialogPrimitive } from "bits-ui";
	import { X } from "@lucide/svelte";
	import { Button } from "../button/index";
	import { cn, type WithoutChildrenOrChild } from "../utils";
	import DialogOverlay from "./dialog-overlay.svelte";
	import DialogPortal from "./dialog-portal.svelte";
	import type { Snippet } from "svelte";
	import type { ComponentProps } from "svelte";
	import { getContext, onMount } from "svelte";
	import type { Readable } from "svelte/store";
	import { windowHost, windowLayerKey, windowTriggerKey } from "./windowStack";

	let {
		ref = $bindable(null),
		class: className,
		portalProps = {},
		children,
		showCloseButton = true,
		overlayClass,
		...restProps
	}: WithoutChildrenOrChild<DialogPrimitive.ContentProps> & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof DialogPortal>>;
		children: Snippet;
		showCloseButton?: boolean;
		overlayClass?: string;
	} = $props();
	const layer = getContext<Readable<number>>(windowLayerKey);
	const trigger = getContext<Readable<HTMLElement | undefined>>(windowTriggerKey);
	let anchor: HTMLSpanElement;
	let host = $state<HTMLElement>();
	onMount(() => { host = windowHost(anchor); });
</script>

<span hidden bind:this={anchor}></span>
{#if host}
<DialogPortal to={host} {...portalProps}>
	<DialogOverlay class={overlayClass} style={`z-index: ${$layer}`} />
	<DialogPrimitive.Content
		bind:ref
		data-slot="dialog-content"
		class={cn(
			"cn-dialog-content",
			className
		)}
		{...restProps}
		style={[restProps.style, `z-index: ${$layer + 1}`].filter(Boolean).join('; ')}
		onCloseAutoFocus={(event) => {
			restProps.onCloseAutoFocus?.(event);
			if (!event.defaultPrevented && $trigger?.isConnected) {
				event.preventDefault();
				$trigger.focus({ preventScroll: true });
			}
		}}
	>
		{@render children?.()}
		{#if showCloseButton}
			<DialogPrimitive.Close data-slot="dialog-close">
				{#snippet child({ props })}
					<Button variant="ghost" class="cn-dialog-close" size="icon-sm" {...props}>
						<X size={17} />
						<span class="sr-only">关闭</span>
					</Button>
				{/snippet}
			</DialogPrimitive.Close>
		{/if}
	</DialogPrimitive.Content>
</DialogPortal>
{/if}
