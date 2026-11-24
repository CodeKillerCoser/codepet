<script lang="ts">
	import { Dialog as DialogPrimitive } from "bits-ui";
	import { setContext } from "svelte";
	import { writable } from "svelte/store";
	import { windowLayerKey, windowStack, windowTriggerKey, windowOpeningTrigger } from "./windowStack";

	let { open = $bindable(false), ...restProps }: DialogPrimitive.RootProps = $props();
	const layer = writable(1000);
	setContext(windowLayerKey, layer);
	const trigger = writable<HTMLElement | undefined>();
	setContext(windowTriggerKey, trigger);
	$effect(() => {
		if (!open) return;
		const window = windowStack.open();
		trigger.set(windowOpeningTrigger());
		layer.set(window.layer);
		return () => window.close();
	});
</script>

<DialogPrimitive.Root bind:open {...restProps} />
