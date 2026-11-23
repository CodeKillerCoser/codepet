<script lang="ts">
  import { Button } from "./ui/button/index";
  import { Check, CheckCircle2, Clock3, Copy, QrCode, RotateCcw, X } from "@lucide/svelte";
  import * as Dialog from "./ui/dialog/index";
  import { pairingCountdownLabel, type PairingCopyStatus, type PairingDisplayState } from "./remoteDevices";

  export let open = false;
  export let display: PairingDisplayState;
  export let onClose: () => void;
  export let onRetry: (() => void | Promise<void>) | undefined = undefined;
  export let canCopyPairingJson = false;
  export let copyStatus: PairingCopyStatus = "idle";
  export let copyMessage: string | null = null;
  export let onCopyPairingJson: (() => void | Promise<void>) | undefined = undefined;

  let closeButton: HTMLButtonElement | null = null;

  function retry() {
    if (onRetry) void onRetry();
  }

  function copyPairingJson() {
    if (onCopyPairingJson) void onCopyPairingJson();
  }

  $: copyButtonLabel = display.phase === "waiting"
    ? copyStatus === "copying"
      ? "复制中"
      : copyStatus === "copied"
        ? "已复制"
        : copyStatus === "unavailable"
          ? "配对 JSON 不可用"
          : copyStatus === "failed"
            ? "重试复制"
            : "复制配对 JSON"
    : display.phase === "expired"
      ? "配对 JSON 已过期"
      : display.phase === "cancelled"
        ? "配对已取消"
        : display.phase === "success"
          ? "配对已完成"
          : "配对 JSON 不可用";
</script>

<Dialog.Root {open} onOpenChange={(next) => { if (!next && open) onClose(); }}>
<Dialog.Content class="pair-device-dialog pixel-panel" showCloseButton={false}
  interactOutsideBehavior="ignore"
  onOpenAutoFocus={(event) => { event.preventDefault(); closeButton?.focus(); }}
  onCloseAutoFocus={(event) => event.preventDefault()}>
  <header class="panel-head">
    <div>
      <span class="agent-kicker">REMOTE CLIENT</span>
      <Dialog.Title id="pair-device-dialog-title">添加设备</Dialog.Title>
    </div>
    <Button bind:ref={closeButton} class="icon-button" type="button" onclick={onClose} aria-label="关闭添加设备对话框">
      <X size={17} />
    </Button>
  </header>

  <Dialog.Description id="pair-device-dialog-description">使用 Remote 客户端扫描二维码，安全连接这台电脑。</Dialog.Description>

  {#if display.phase === "success"}
    <div class="pair-device-success" role="status">
      <CheckCircle2 size={40} />
      <strong>设备已连接</strong>
      <p>{display.pairedClientName ? `${display.pairedClientName} 已完成安全配对。` : "Remote 客户端已完成安全配对。"}</p>
    </div>
  {:else}
    <div class:expired={display.phase === "expired" || display.phase === "cancelled" || display.phase === "error"} class="pair-device-qr-frame">
      {#if display.phase === "waiting" && display.qrImageUrl}
        <img src={display.qrImageUrl} alt="设备配对二维码" />
      {:else}
        <QrCode size={48} />
        <strong>
          {display.phase === "expired"
            ? "二维码已过期"
            : display.phase === "cancelled"
              ? "配对已取消"
              : display.phase === "error"
                ? "无法生成二维码"
                : display.phase === "starting" || display.phase === "waiting"
                  ? "正在生成二维码"
                  : "Remote Host 尚未就绪"}
        </strong>
        <span>{display.phase === "starting" || display.phase === "waiting" ? "请稍候" : "可重试生成新的安全配对二维码"}</span>
      {/if}
    </div>

    {#if display.errorMessage}
      <div class="pair-device-error" role="alert">
        <span>{display.errorMessage}</span>
      </div>
    {/if}

    {#if copyMessage}
      <div class="pair-device-copy-feedback" class:failed={copyStatus === "failed" || copyStatus === "unavailable"} role={copyStatus === "failed" || copyStatus === "unavailable" ? "alert" : "status"}>
        {copyStatus === "copied" ? "✓" : "!"} {copyMessage}
      </div>
    {/if}

    <div class="pair-device-expiry" role="status" title={display.expiresAtMs ? new Date(display.expiresAtMs).toLocaleString("zh-CN") : undefined}>
      <Clock3 size={16} />
      {#if display.phase === "waiting" && display.remainingSeconds != null}
        <span>有效期 {pairingCountdownLabel(display.remainingSeconds)}</span>
      {:else if display.phase === "expired"}
        <span>当前二维码已失效</span>
      {:else if display.phase === "cancelled"}
        <span>当前配对已取消</span>
      {:else if display.phase === "starting"}
        <span>正在获取有效期</span>
      {:else if display.phase === "waiting"}
        <span>正在获取有效期</span>
      {:else}
        <span>有效期将在配对信息生成后显示</span>
      {/if}
    </div>
  {/if}

  <div class="row-actions pair-device-dialog-actions">
    <Button class={["pair-device-copy-button", (copyStatus === "copied") ? "copied" : ''].filter(Boolean).join(' ')}


      type="button"
      disabled={!canCopyPairingJson || copyStatus === "copying" || copyStatus === "unavailable"}
      onclick={copyPairingJson}
    >
      {#if copyStatus === "copied" && display.phase === "waiting"}
        <Check size={16} />
      {:else}
        <Copy size={16} />
      {/if}
      {copyButtonLabel}
    </Button>
    {#if (display.phase === "expired" || display.phase === "cancelled" || display.phase === "error") && onRetry}
      <Button type="button" onclick={retry}><RotateCcw size={16} /> 重新生成</Button>
    {/if}
    <Button type="button" onclick={onClose}>{display.phase === "success" ? "完成" : "关闭"}</Button>
  </div>
</Dialog.Content>
</Dialog.Root>
