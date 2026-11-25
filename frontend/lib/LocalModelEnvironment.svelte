<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import {
    Box,
    Check,
    Download,
    ExternalLink,
    LoaderCircle,
    RefreshCw,
    X,
  } from "@lucide/svelte";
  import * as Dialog from "./ui/dialog/index";
  import {
    runtimeRequest,
    runtimeActive,
    byteSize,
    type RuntimeStatus,
    type RuntimePlan,
  } from "./localModelRuntime";
  export let onUseLocal: (endpoint: string) => void;
  let status: RuntimeStatus | null = null,
    loading = true,
    busy = false,
    error = "";
  let confirmation = false,
    plan: RuntimePlan | null = null,
    preparing = false,
    confirmError = "",
    confirmBusy = false;
  let timer: ReturnType<typeof setTimeout> | undefined,
    disposed = false,
    planSequence = 0,
    requestSequence = 0;
  let cancelButton: HTMLButtonElement, primaryButton: HTMLButtonElement;
  let restoreAfterCancel = false;
  async function restoreFocus() {
    if (
      !restoreAfterCancel ||
      busy ||
      loading ||
      runtimeActive(status?.job ?? null)
    )
      return;
    await tick();
    if (disposed) return;
    restoreAfterCancel = false;
    if (
      document.activeElement === document.body ||
      document.activeElement === cancelButton
    )
      primaryButton?.focus({ preventScroll: true });
  }
  $: active = runtimeActive(status?.job ?? null);
  $: percent = status?.job?.totalBytes
    ? Math.min(
        100,
        Math.floor((status.job.downloadedBytes / status.job.totalBytes) * 100),
      )
    : 0;
  async function refresh(quiet = false) {
    clearTimeout(timer);
    const sequence = ++requestSequence;
    if (!quiet) loading = true;
    try {
      const value = await runtimeRequest();
      if (!disposed && sequence === requestSequence) {
        const restore = document.activeElement === cancelButton;
        status = value;
        error = "";
        if (restore && !runtimeActive(value.job)) {
          await tick();
          primaryButton?.focus({ preventScroll: true });
        }
      }
    } catch (cause) {
      if (!disposed && sequence === requestSequence)
        error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      if (!disposed && sequence === requestSequence) {
        loading = false;
        void restoreFocus();
        if (runtimeActive(status?.job ?? null))
          timer = setTimeout(() => void refresh(true), 1000);
      }
    }
  }
  async function prepare() {
    confirmation = true;
    plan = null;
    confirmError = "";
    preparing = true;
    const sequence = ++planSequence;
    try {
      const value = await runtimeRequest<RuntimePlan>("prepare");
      if (!disposed && sequence === planSequence && confirmation) plan = value;
    } catch (cause) {
      if (!disposed && sequence === planSequence)
        confirmError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      if (!disposed && sequence === planSequence) preparing = false;
    }
  }
  function dismiss() {
    if (confirmBusy) return;
    confirmation = false;
    planSequence++;
    preparing = false;
  }
  async function install() {
    if (!plan || confirmBusy) return;
    confirmBusy = true;
    confirmError = "";
    try {
      const value = await runtimeRequest("install", plan.id);
      if (disposed) return;
      status = value;
      confirmation = false;
      await tick();
      cancelButton?.focus();
      void refresh(true);
    } catch (cause) {
      if (!disposed)
        confirmError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      if (!disposed) confirmBusy = false;
    }
  }
  async function act(action: "open" | "cancel") {
    if (busy) return;
    if (action === "cancel") restoreAfterCancel = true;
    busy = true;
    error = "";
    requestSequence++;
    clearTimeout(timer);
    try {
      const restore = document.activeElement === cancelButton;
      const value = await runtimeRequest(
        action,
        action === "cancel" ? status?.job?.id : undefined,
      );
      if (!disposed) {
        status = value;
        if (restore && !runtimeActive(value.job)) {
          await tick();
          primaryButton?.focus({ preventScroll: true });
        }
        void refresh(true);
      }
    } catch (cause) {
      if (!disposed) {
        error = cause instanceof Error ? cause.message : String(cause);
        if (active) timer = setTimeout(() => void refresh(true), 1000);
      }
    } finally {
      if (!disposed) {
        busy = false;
        void restoreFocus();
      }
    }
  }
  onMount(() => void refresh());
  onDestroy(() => {
    disposed = true;
    requestSequence++;
    planSequence++;
    clearTimeout(timer);
  });
</script>

<section class="model-environment" aria-label="本地运行环境">
  <div class="model-section-heading">
    <h3>本地运行环境</h3>
    <span>在 Mac 上运行模型，任务内容留在本机</span>
  </div>
  <div class="model-runtime-row">
    <div class="model-runtime-icon"><Box size={24} /></div>
    <div class="model-runtime-copy">
      <strong>MLX</strong>
      {#if loading}<p class="model-hint" role="status">正在检测本机环境…</p>
      {:else if status?.installation}<p class="model-hint">
          <Check size={13} />{status.installation.name} · {status.installation
            .version} · {status.installation.source === "system"
            ? "使用系统已有应用"
            : "由 CodePet 安装"}
        </p>
      {:else if status && !status.supported}<p class="model-hint">
          {status.reason}
        </p>
      {:else}<p class="model-hint">
          未检测到 MLX。安装运行环境后，可在应用中下载语言模型和向量模型。
        </p>{/if}
    </div>
    <div class="model-runtime-actions">
      <button
        type="button"
        class="model-icon"
        aria-label="重新检测本地环境"
        title="重新检测"
        disabled={loading || busy || active}
        on:click={() => refresh()}
      >
        <RefreshCw size={16} />
      </button>
      {#if status?.installation}<button
          type="button"
          bind:this={primaryButton}
          disabled={busy}
          on:click={() => act("open")}
        >
          <ExternalLink size={14} />打开 MLX
        </button>
      {:else if status?.supported && !active}<button
          type="button"
          bind:this={primaryButton}
          disabled={busy || loading}
          on:click={prepare}
        >
          <Download size={14} />{status.job?.state === "failed"
            ? "重新安装"
            : "安装 MLX"}
        </button>{/if}
    </div>
  </div>
  {#if status?.installation}<div class="model-runtime-footer">
      <span title={status.installation.path}>
        {status.installation.path}
      </span><button
        type="button"
        class="model-text-button"
        on:click={() => onUseLocal(status!.endpoint)}
      >
        使用本地 API
      </button>
    </div>{/if}
  {#if status?.job && active}<div
      class="model-install-progress"
      aria-label="MLX 安装进度"
    >
      <div>
        <span role="status">{status.job.stage}</span><button
          type="button"
          class="model-text-button"
          bind:this={cancelButton}
          disabled={busy || !status.job.canCancel}
          on:click={() => act("cancel")}
        >
          {status.job.state === "cancelling" ? "正在取消…" : "取消安装"}
        </button>
      </div>
      <progress
        max="100"
        value={status.job.state === "downloading" || percent === 100
          ? percent
          : undefined}
        aria-label="安装进度"
      ></progress>
      <p class="model-hint">
        {status.job.state === "downloading"
          ? `${byteSize(status.job.downloadedBytes)} / ${byteSize(status.job.totalBytes)} · ${percent}%`
          : "请稍候，完成后会自动更新状态。"}
      </p>
    </div>
  {:else if status?.job?.state === "cancelled"}<p
      class="model-hint"
      role="status"
    >
      已取消安装，可随时重新开始。
    </p>
  {:else if status?.job?.state === "failed"}<p
      class="model-field-error"
      role="alert"
    >
      {status.job.error}
    </p>{/if}
  {#if error}<p class="model-field-error" role="alert">{error}</p>{/if}
</section>

<Dialog.Root
  open={confirmation}
  onOpenChange={(value) => {
    if (!value) dismiss();
  }}
>
  <Dialog.Content
    class="model-install-dialog"
    showCloseButton={false}
    onEscapeKeydown={(event) => {
      if (confirmBusy) event.preventDefault();
    }}
    onInteractOutside={(event) => {
      if (confirmBusy) event.preventDefault();
    }}
  >
    <header>
      <Dialog.Title>安装 MLX 运行环境</Dialog.Title><button
        type="button"
        class="model-icon"
        aria-label="关闭安装确认"
        disabled={confirmBusy}
        on:click={dismiss}
      >
        <X size={18} />
      </button>
    </header>
    <Dialog.Description>
      从 MLX-Serve 官方发布页下载安装包，不包含模型权重。安装完成后，在 MLX
      中下载所需模型。
    </Dialog.Description>
    {#if preparing}<p role="status">
        <LoaderCircle size={16} class="model-spinning" />正在获取官方安装信息…
      </p>{/if}
    {#if plan}<dl>
        <div>
          <dt>版本</dt>
          <dd>{plan.version}</dd>
        </div>
        <div>
          <dt>下载大小</dt>
          <dd>{byteSize(plan.bytes)}</dd>
        </div>
        <div>
          <dt>安装方式</dt>
          <dd>安装到 CodePet 管理目录，无需管理员权限</dd>
        </div>
      </dl>
      <a href={plan.sourceUrl} target="_blank" rel="noreferrer">
        查看官方发布页
      </a>{/if}
    {#if confirmError}<p role="alert">{confirmError}</p>{/if}
    <footer>
      <button type="button" disabled={confirmBusy} on:click={dismiss}>
        取消
      </button>{#if confirmError && !plan}<button
          type="button"
          disabled={preparing}
          on:click={prepare}
        >
          重试
        </button>{:else}<button
          type="button"
          disabled={!plan || preparing || confirmBusy}
          on:click={install}
        >
          {confirmBusy ? "正在开始…" : "确认安装"}
        </button>{/if}
    </footer>
  </Dialog.Content>
</Dialog.Root>
