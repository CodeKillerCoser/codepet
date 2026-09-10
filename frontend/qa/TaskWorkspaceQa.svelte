<script lang="ts">
  import TaskLineage from "../lib/TaskLineage.svelte";
  import TaskSettingsPage from "../lib/TaskSettingsPage.svelte";
  import WindowToolbar from "../lib/WindowToolbar.svelte";
  let tab = "tasks", collapsed = false, error = "";
</script>
<main class="app-shell main-theme light" class:sidebar-collapsed={collapsed}>
  <WindowToolbar bind:collapsed onError={message => error = message} />
  <aside id="main-sidebar" class="sidebar" inert={collapsed}>
    <div class="brand"><h1>Code Pet</h1></div>
    <nav class="tabs" aria-label="主导航"><button class:active={tab === "tasks"} on:click={() => tab = "tasks"}>任务</button><button class:active={tab === "settings"} on:click={() => tab = "settings"}>设置</button></nav>
  </aside>
  <section class="content-pane"><header class="topbar"><h2>{tab === "tasks" ? "任务管理" : "设置"}</h2></header><div class="content">{#if tab === "tasks"}<TaskLineage />{:else}<TaskSettingsPage />{/if}{#if error}<p role="alert">{error}</p>{/if}</div></section>
</main>
