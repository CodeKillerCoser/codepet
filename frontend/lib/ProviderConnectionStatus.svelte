<script lang="ts">
  import type { ProviderConnectionState } from "./providerRuntimes";

  export let current: ProviderConnectionState | undefined;
  export let available = false;
  const labels: Record<string, string> = {
    created: "未启动", starting: "启动中", ready: "可用",
    stopping: "停止中", stopped: "已停止", error: "错误",
  };
  $: connection = current?.connectionStatus;
</script>

<div class="provider-connection" aria-live="polite">
  <span class:online={connection === "online"}>
    Provider · {connection === "online" ? "在线" : connection === "connecting" ? "连接中" : connection === "offline" ? "离线" : available ? "未启动" : "状态未知"}
  </span>
  {#if current}
    {#each current.instances as instance}
      <span>Harness · {labels[instance.status] ?? instance.status}</span>
    {/each}
  {/if}
</div>

<style>
  .provider-connection { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 12px; margin-bottom: 12px; }
  .online { color: var(--color-success); }
</style>
