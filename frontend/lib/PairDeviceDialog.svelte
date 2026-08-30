<script lang="ts">
  import { CheckCircle2, Clock3, QrCode, RotateCcw, X } from "@lucide/svelte";
  import { tick } from "svelte";
  import { wrappedDialogFocusIndex } from "./dialogFocus";
  import { pairingCountdownLabel, type PairingDisplayState } from "./remoteDevices";

  export let open = false;
  export let display: PairingDisplayState;
  export let onClose: () => void;
  export let onRetry: (() => void | Promise<void>) | undefined = undefined;

  let dialogElement: HTMLDialogElement | null = null;
  let closeButton: HTMLButtonElement | null = null;

  $: if (open && dialogElement && !dialogElement.open) {
    dialogElement.showModal();
    void focusCloseButton();
  }

  $: if (!open && dialogElement && dialogElement.open) {
    dialogElement.close();
  }

  async function focusCloseButton() {
    await tick();
    closeButton?.focus();
  }

  function handleCancel(event: Event) {
    event.preventDefault();
    onClose();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }

    if (event.key !== "Tab" || !dialogElement) return;

    const focusableElements = Array.from(
      dialogElement.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    const activeIndex = focusableElements.indexOf(document.activeElement as HTMLElement);
    const targetIndex = wrappedDialogFocusIndex(activeIndex, focusableElements.length, event.shiftKey);
    if (targetIndex === null) return;

    event.preventDefault();
    focusableElements[targetIndex]?.focus();
  }

  function retry() {
    if (onRetry) void onRetry();
  }
</script>

<dialog
  bind:this={dialogElement}
  class="pair-device-dialog pixel-panel"
  aria-labelledby="pair-device-dialog-title"
  aria-describedby="pair-device-dialog-description"
  on:cancel={handleCancel}
  on:keydown={handleKeydown}
>
  <header class="panel-head">
    <div>
      <span class="agent-kicker">REMOTE CLIENT</span>
      <h3 id="pair-device-dialog-title">添加设备</h3>
    </div>
    <button bind:this={closeButton} class="icon-button" type="button" on:click={onClose} aria-label="关闭添加设备对话框">
      <X size={17} />
    </button>
  </header>

  <p id="pair-device-dialog-description">配对服务接入后，可使用 Remote 客户端扫描二维码连接这台电脑。</p>

  {#if display.phase === "success"}
    <div class="pair-device-success" role="status">
      <CheckCircle2 size={40} />
      <strong>设备已连接</strong>
      <p>{display.pairedClientName ? `${display.pairedClientName} 已完成安全配对。` : "Remote 客户端已完成安全配对。"}</p>
    </div>
  {:else}
    <div class:expired={display.phase === "expired"} class="pair-device-qr-frame">
      {#if display.phase === "waiting" && display.qrImageUrl}
        <img src={display.qrImageUrl} alt="设备配对二维码" />
      {:else}
        <QrCode size={48} />
        <strong>{display.phase === "expired" ? "二维码已过期" : display.phase === "waiting" ? "正在生成二维码" : "等待配对服务接入"}</strong>
        <span>{display.phase === "unavailable" ? "此处不会显示配对凭据明文" : "请重新生成安全配对二维码"}</span>
      {/if}
    </div>

    <div class="pair-device-expiry" role="status" title={display.expiresAtMs ? new Date(display.expiresAtMs).toLocaleString("zh-CN") : undefined}>
      <Clock3 size={16} />
      {#if display.phase === "waiting" && display.remainingSeconds != null}
        <span>有效期 {pairingCountdownLabel(display.remainingSeconds)}</span>
      {:else if display.phase === "expired"}
        <span>当前二维码已失效</span>
      {:else if display.phase === "waiting"}
        <span>正在获取有效期</span>
      {:else}
        <span>有效期将在配对信息生成后显示</span>
      {/if}
    </div>
  {/if}

  <div class="row-actions pair-device-dialog-actions">
    {#if display.phase === "expired" && onRetry}
      <button type="button" on:click={retry}><RotateCcw size={16} /> 重新生成</button>
    {/if}
    <button type="button" on:click={onClose}>{display.phase === "success" ? "完成" : "关闭"}</button>
  </div>
</dialog>
