<script lang="ts">
  import { onMount } from "svelte";
  import { Copy, Minus, PanelLeft, Square, X } from "@lucide/svelte";
  import { createWindowChrome } from "./windowChrome";

  export let collapsed = false;
  export let onError: (message: string) => void;
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
  <button class="window-toolbar-button" type="button" aria-label={collapsed ? "展开导航栏" : "收起导航栏"} aria-expanded={!collapsed} aria-controls="main-sidebar" on:click={() => collapsed = !collapsed}>
    <PanelLeft size={16} strokeWidth={1.75} />
  </button>
  <div class="window-drag-space" data-tauri-drag-region></div>
  {#if chrome.platform === "windows" && chrome.native}
    <div class="window-controls">
      <button type="button" aria-label="最小化" on:click={() => perform(chrome.minimize)}><Minus size={14} /></button>
      <button type="button" aria-label={maximized ? "还原窗口" : "最大化"} on:click={() => perform(chrome.toggleMaximize)}>{#if maximized}<Copy size={12} />{:else}<Square size={12} />{/if}</button>
      <button class="window-close" type="button" aria-label="关闭窗口" on:click={() => perform(chrome.close)}><X size={16} /></button>
    </div>
  {/if}
</div>
