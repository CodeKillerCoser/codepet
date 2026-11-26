<script lang="ts">
  import { Button } from "./ui/button/index";
  import { LayoutGrid, GitBranch, Brain, Bot, MonitorSmartphone, BarChart3, Palette, Cat, Settings, Bell, Cpu, Wrench, Info, ShieldCheck } from "@lucide/svelte";
  import logo from "../../src-tauri/icons/icon.png";
  import { isConnectionRoute, isSettingsRoute, settingsRoutes, type AppRoute } from "./navigation";
  export let current: AppRoute;
  export let onNavigate: (route: AppRoute) => void;
  const entries = [
    { route: "tasks", label: "工作台", icon: LayoutGrid },
    { route: "worktrees", label: "工作树", icon: GitBranch },
    { route: "knowledge", label: "工作记忆", icon: Brain },
    { route: "connections", label: "智能体", icon: Bot },
    { route: "devices", label: "远程控制", icon: MonitorSmartphone },
    { route: "usage", label: "用量", icon: BarChart3 },
  ] as const;
  const settingIcons = { settings: Settings, appearance: Palette, personalize: Cat, notifications: Bell, models: Cpu, permissions: ShieldCheck, events: Wrench, about: Info };
  $: inSettings = isSettingsRoute(current);
</script>

<div class="brand">
  {#if !inSettings}<img class="brand-mark" src={logo} alt="" width="20" height="20" aria-hidden="true" />{/if}
  <h1>{inSettings ? "设置" : "Code Pet"}</h1>
</div>
{#if inSettings}
  <nav class="tabs" aria-label="设置分区">
    {#each settingsRoutes as entry}
      <Button class={['ui-selectable', current === entry.route ? 'active selected' : ''].filter(Boolean).join(' ')}  aria-current={current === entry.route ? "page" : undefined} onclick={() => onNavigate(entry.route)}><svelte:component this={settingIcons[entry.route]} size={16} />{entry.label}</Button>
    {/each}
  </nav>
{:else}
  <nav class="tabs" aria-label="主导航">
    {#each entries as entry}
      {@const active = current === entry.route || (entry.route === "worktrees" && current === "extraction") || (entry.route === "connections" && isConnectionRoute(current))}
      <Button class={['ui-selectable', active ? 'active selected' : ''].filter(Boolean).join(' ')}  aria-current={active ? "page" : undefined} onclick={() => onNavigate(entry.route)}><svelte:component this={entry.icon} size={16} />{entry.label}</Button>
    {/each}
  </nav>
{/if}
