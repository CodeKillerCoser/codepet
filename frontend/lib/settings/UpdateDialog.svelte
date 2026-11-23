<script lang="ts">
  import { Download, X } from "@lucide/svelte";
  import * as Dialog from "../ui/dialog/index";
  import { Button } from "../ui/button/index";
  import type { AppUpdate } from "../types";
  export let update: AppUpdate | null;
  export let busy = false;
  export let error = "";
  export let onDismiss: () => void | Promise<void>;
  export let onInstall: () => void | Promise<void>;
</script>

<Dialog.Root open={!!update} onOpenChange={(next) => { if (!next && !busy && update) void onDismiss(); }}>
  <Dialog.Content class="update-dialog pixel-panel" showCloseButton={false}
    interactOutsideBehavior="ignore" escapeKeydownBehavior={busy ? "ignore" : "close"}>
    {#if update}
      <header class="panel-head">
        <div><Dialog.Title id="update-dialog-title"><Download size={18} /> 发现新版本</Dialog.Title></div>
        <Button class="icon-button" disabled={busy} onclick={onDismiss} aria-label="取消更新"><X size={17} /></Button>
      </header>
      <Dialog.Description>发现 Code Pet {update.version}，当前版本为 {update.currentVersion}。</Dialog.Description>
      {#if update.manualDownload}<p>此版本通过 DMG 安装。点击下载后，请打开安装包手动安装。</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="row-actions">
        <Button disabled={busy} onclick={onDismiss}>取消</Button>
        <Button class="primary-action" disabled={busy} onclick={onInstall}>
          <Download size={17} /> {update.manualDownload ? (busy ? "正在打开…" : "下载 DMG") : (busy ? "正在安装…" : "立即升级")}
        </Button>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
