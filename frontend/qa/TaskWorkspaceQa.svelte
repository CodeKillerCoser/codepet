<script lang="ts">
  import TaskLineage from "../lib/TaskLineage.svelte";
  import TaskSettingsPage from "../lib/TaskSettingsPage.svelte";
  import WindowToolbar from "../lib/WindowToolbar.svelte";
  import MainNavigation from "../lib/MainNavigation.svelte";
  import { createNavigation, isSettingsRoute, routeTitle } from "../lib/navigation";
  const navigation = createNavigation("tasks");
  $: tab = $navigation.current;
  let collapsed = false, error = "";
</script>
<main class="app-shell main-theme light" class:sidebar-collapsed={collapsed}>
  <WindowToolbar bind:collapsed canGoBack={$navigation.canGoBack} canGoForward={$navigation.canGoForward} onBack={navigation.back} onForward={navigation.forward} settingsActive={isSettingsRoute(tab)} onSettings={() => navigation.navigate("extraction")} onError={message => error = message} />
  <aside id="main-sidebar" class="sidebar" inert={collapsed}>
    <MainNavigation current={tab} onNavigate={navigation.navigate} />
  </aside>
  <section class="content-pane"><header class="topbar"><h2>{routeTitle(tab)}</h2></header><div class="content">{#if tab === "tasks"}<TaskLineage onSettings={() => navigation.navigate("extraction")} />{:else if tab === "extraction"}<TaskSettingsPage />{:else}<p>其他页面的内容未包含在此测试场景中。</p>{/if}{#if error}<p role="alert">{error}</p>{/if}</div></section>
</main>
