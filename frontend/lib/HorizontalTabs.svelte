<script lang="ts" generics="T extends string">
  import * as Tabs from "./ui/tabs/index";
  export let items: readonly { route: T; label: string }[];
  export let value: T;
  export let label: string;
  export let panelId: string;
  export let onChange: (value: T) => void;
</script>

<Tabs.Root {value} onValueChange={(next) => onChange(next as T)}>
  <Tabs.List class="horizontal-tabs" aria-label={label}>
    {#each items as item}
      <Tabs.Trigger value={item.route} id={`tab-${item.route}`} aria-controls={panelId}>{item.label}</Tabs.Trigger>
    {/each}
  </Tabs.List>
</Tabs.Root>
