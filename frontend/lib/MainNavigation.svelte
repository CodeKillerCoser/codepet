<script lang="ts">
  import { Button } from "./ui/button/index";
  import { Activity, Cable, BarChart3, Palette, Settings, Bell, ListTree, Wrench, Info, ShieldCheck, Network } from "@lucide/svelte";
  import { isConnectionRoute, isSettingsRoute, settingsRoutes, type AppRoute } from "./navigation";
  export let current: AppRoute;
  export let onNavigate: (route: AppRoute) => void;
  const entries = [
    { route: "tasks", label: "AgentHub", icon: Activity },
    { route: "knowledge", label: "任务图谱", icon: Network },
    { route: "connections", label: "连接接入", icon: Cable },
    { route: "usage", label: "用量", icon: BarChart3 },
    { route: "personalize", label: "宠物制作", icon: Palette },
  ] as const;
  const settingIcons = { settings: Settings, models: Cable, appearance: Palette, notifications: Bell, permissions: ShieldCheck, extraction: ListTree, events: Wrench, about: Info };
  $: inSettings = isSettingsRoute(current);
</script>

<div class="brand"><h1>{inSettings ? "设置" : "Code Pet"}</h1></div>
{#if inSettings}
  <nav class="tabs" aria-label="设置分区">
    {#each settingsRoutes as entry}
      <Button class={[(current === entry.route) ? "active" : ''].filter(Boolean).join(' ')}  aria-current={current === entry.route ? "page" : undefined} onclick={() => onNavigate(entry.route)}><svelte:component this={settingIcons[entry.route]} size={18} />{entry.label}</Button>
    {/each}
  </nav>
{:else}
  <nav class="tabs" aria-label="主导航">
    {#each entries as entry}
      {@const active = current === entry.route || (entry.route === "connections" && isConnectionRoute(current))}
      <Button class={[(active) ? "active" : ''].filter(Boolean).join(' ')}  aria-current={active ? "page" : undefined} onclick={() => onNavigate(entry.route)}><svelte:component this={entry.icon} size={18} />{entry.label}</Button>
    {/each}
  </nav>
{/if}
