<script lang="ts">
  import { Button } from "./ui/button/index";
  import { onMount } from "svelte";
  import { ArrowLeft, ArrowRight, Copy, Home, Minus, PanelLeft, Settings, Square, X } from "@lucide/svelte";
  import { createWindowChrome } from "./windowChrome";

  export let collapsed = false;
  export let onError: (message: string) => void;
  export let canGoBack = false;
  export let canGoForward = false;
  export let onHome: () => void = () => {};
  export let onBack: () => void = () => {};
  export let onForward: () => void = () => {};
  export let settingsActive = false;
  export let onSettings: () => void = () => {};
  const chrome = createWindowChrome();
  let maximized = false;
  let anchor = { left: 88, centerY: 20 };

  async function perform(action: () => Promise<void>) {
    try { await action(); } catch (error) { onError(String(error)); }
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void chrome.initialize((value) => { if (!disposed) maximized = value; }, (error) => { if (!disposed) onError(String(error)); }, (value) => { if (!disposed) anchor = value; })
      .then((cleanup) => { if (disposed) cleanup(); else unlisten = cleanup; })
      .catch((error) => onError(String(error)));
    return () => { disposed = true; unlisten?.(); };
  });
</script>

<div class="window-toolbar" class:macos={chrome.platform === "macos"} style:--toolbar-anchor-left={`${anchor.left}px`} style:--toolbar-anchor-y={`${anchor.centerY}px`} data-tauri-drag-region>
  <div class="window-navigation-controls">
  <Button class="window-toolbar-button" type="button" aria-label={collapsed ? "展开导航栏" : "收起导航栏"} aria-expanded={!collapsed} aria-controls="main-sidebar" onclick={() => collapsed = !collapsed}>
    <PanelLeft size={16} strokeWidth={1.75} />
  </Button>
  <Button class="window-toolbar-button" type="button" aria-label="返回主界面" title="返回主界面" onclick={onHome}><Home size={16} /></Button>
  <Button class="window-toolbar-button" type="button" aria-label="后退" title="后退" disabled={!canGoBack} onclick={onBack}><ArrowLeft size={16} /></Button>
  <Button class="window-toolbar-button" type="button" aria-label="前进" title="前进" disabled={!canGoForward} onclick={onForward}><ArrowRight size={16} /></Button>
  </div>
  <div class="window-drag-space" data-tauri-drag-region></div>
  <Button class="window-toolbar-button settings-entry" type="button" aria-label="设置" title="设置" aria-pressed={settingsActive} onclick={onSettings}><Settings size={18} /></Button>
  {#if chrome.platform === "windows" && chrome.native}
    <div class="window-controls">
      <Button type="button" aria-label="最小化" onclick={() => perform(chrome.minimize)}><Minus size={14} /></Button>
      <Button type="button" aria-label={maximized ? "还原窗口" : "最大化"} onclick={() => perform(chrome.toggleMaximize)}>{#if maximized}<Copy size={12} />{:else}<Square size={12} />{/if}</Button>
      <Button class="window-close" type="button" aria-label="关闭窗口" onclick={() => perform(chrome.close)}><X size={16} /></Button>
    </div>
  {/if}
</div>
