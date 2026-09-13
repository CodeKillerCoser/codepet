<script lang="ts" generics="T extends string">
  import { tick } from "svelte";
  export let items: readonly { route: T; label: string }[];
  export let value: T;
  export let label: string;
  export let panelId: string;
  export let onChange: (value: T) => void;
  let element: HTMLDivElement;
  async function handleKey(event: KeyboardEvent, index: number) {
    let next: number;
    if (event.key === "ArrowRight") next = (index + 1) % items.length;
    else if (event.key === "ArrowLeft") next = (index + items.length - 1) % items.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    else return;
    event.preventDefault();
    onChange(items[next].route);
    await tick();
    element.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
  }
</script>

<div class="horizontal-tabs" role="tablist" aria-label={label} aria-orientation="horizontal" bind:this={element}>
  {#each items as item, index}
    <button type="button" role="tab" id={`tab-${item.route}`} aria-selected={value === item.route}
      aria-controls={panelId} tabindex={value === item.route ? 0 : -1}
      on:click={() => onChange(item.route)} on:keydown={(event) => handleKey(event, index)}>{item.label}</button>
  {/each}
</div>

<style>
  .horizontal-tabs{display:flex;gap:24px;border-bottom:1px solid var(--app-border);margin-bottom:24px;overflow-x:auto;flex-shrink:0}
  .horizontal-tabs button{position:relative;flex-shrink:0;border:0;border-radius:0;background:transparent;color:var(--app-muted);font:inherit;padding:12px 0 14px;box-shadow:none}
  .horizontal-tabs button[aria-selected=true]{color:var(--app-text);font-weight:600}
  .horizontal-tabs button[aria-selected=true]::after{content:"";position:absolute;bottom:0;left:0;right:0;height:2px;background:var(--app-text);border-radius:2px}
  .horizontal-tabs button:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:-3px}
</style>
