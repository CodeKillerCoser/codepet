<script lang="ts">
  import { Activity, Cable, BarChart3, Palette, ArrowLeft, Settings, Bell, ListTree, Wrench } from "@lucide/svelte";
  import { isConnectionRoute, isSettingsRoute, settingsRoutes, type AppRoute } from "./navigation";
  export let current: AppRoute;
  export let onNavigate: (route: AppRoute) => void;
  export let onReturn: () => void = () => onNavigate("tasks");
  const entries = [
    { route: "tasks", label: "任务", icon: Activity },
    { route: "connections", label: "连接接入", icon: Cable },
    { route: "usage", label: "用量", icon: BarChart3 },
    { route: "personalize", label: "宠物制作", icon: Palette },
  ] as const;
  const settingIcons = { settings: Settings, appearance: Palette, notifications: Bell, extraction: ListTree, events: Wrench };
  $: inSettings = isSettingsRoute(current);
</script>

<div class="brand"><h1>{inSettings ? "设置" : "Code Pet"}</h1></div>
{#if inSettings}
  <nav class="tabs" aria-label="设置分区">
    <button class="return-workspace" on:click={onReturn}><ArrowLeft size={18} />返回主界面</button>
    {#each settingsRoutes as entry}
      <button class:active={current === entry.route} aria-current={current === entry.route ? "page" : undefined} on:click={() => onNavigate(entry.route)}><svelte:component this={settingIcons[entry.route]} size={18} />{entry.label}</button>
    {/each}
  </nav>
{:else}
  <nav class="tabs" aria-label="主导航">
    {#each entries as entry}
      {@const active = current === entry.route || (entry.route === "connections" && isConnectionRoute(current))}
      <button class:active aria-current={active ? "page" : undefined} on:click={() => onNavigate(entry.route)}><svelte:component this={entry.icon} size={18} />{entry.label}</button>
    {/each}
  </nav>
{/if}
