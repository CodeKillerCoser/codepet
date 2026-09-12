<script lang="ts">
  import { Activity, Bot, Cable, BarChart3, Palette, ListTree } from "@lucide/svelte";
  import type { AppRoute } from "./navigation";
  export let current: AppRoute;
  export let onNavigate: (route: AppRoute) => void;
  const entries = [
    { route: "tasks", label: "任务", icon: Activity },
    { route: "agents", label: "Agent", icon: Bot },
    { route: "connections", label: "连接", icon: Cable },
    { route: "usage", label: "用量", icon: BarChart3 },
    { route: "personalize", label: "个性化", icon: Palette },
    { route: "events", label: "事件", icon: Activity },
  ] as const;
</script>

<div class="brand"><h1>{current === "settings" ? "设置" : "Code Pet"}</h1></div>
{#if current === "settings"}
  <nav class="tabs" aria-label="设置分区"><button class="active" aria-current="page" on:click={() => onNavigate("settings")}><ListTree size={18} />任务抽取</button></nav>
{:else}
  <nav class="tabs" aria-label="主导航">
    {#each entries as entry}<button class:active={current === entry.route} aria-current={current === entry.route ? "page" : undefined} on:click={() => onNavigate(entry.route)}><svelte:component this={entry.icon} size={18} />{entry.label}</button>{/each}
  </nav>
{/if}
