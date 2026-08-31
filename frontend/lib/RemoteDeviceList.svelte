<script lang="ts">
  import { CircleHelp, Globe2, Monitor, Smartphone, Tablet, Trash2 } from "@lucide/svelte";
  import { remoteDeviceConnectionLabel, remoteDeviceStatusMeta, remoteDeviceSystemLabel, type RemoteDevice } from "./remoteDevices";

  export let devices: RemoteDevice[] = [];
  export let nowMs = Date.now();
  export let loading = false;
  export let unavailable = false;
  export let revokingDeviceId: string | null = null;
  export let onRevoke: ((device: RemoteDevice) => void | Promise<void>) | undefined = undefined;

  function revoke(device: RemoteDevice) {
    if (!onRevoke || device.status === "revoked") return;
    void onRevoke(device);
  }
</script>

{#if devices.length}
  <div class="remote-device-list" aria-label="已配对 Remote 客户端">
    {#each devices as device (device.id)}
      {@const status = remoteDeviceStatusMeta(device.status)}
      <article class="remote-device-row">
        <span class="remote-device-icon" aria-hidden="true">
          {#if device.deviceType === "phone"}
            <Smartphone size={22} />
          {:else if device.deviceType === "tablet"}
            <Tablet size={22} />
          {:else if device.deviceType === "desktop"}
            <Monitor size={22} />
          {:else if device.deviceType === "browser"}
            <Globe2 size={22} />
          {:else}
            <CircleHelp size={22} />
          {/if}
        </span>

        <div class="remote-device-copy">
          <strong>{device.deviceName}</strong>
          <span class="remote-device-platform">{remoteDeviceSystemLabel(device)}</span>
          <span>{remoteDeviceConnectionLabel(device, nowMs)}</span>
        </div>

        <div class="remote-device-actions">
          <span class:online={status.tone === "ready"} class:runtime-danger={status.tone === "danger"} class="status-chip">
            {status.label}
          </span>
          <button
            class="remote-device-revoke"
            type="button"
            disabled={!onRevoke || revokingDeviceId === device.id || device.status === "revoked"}
            on:click={() => revoke(device)}
            aria-label={`撤销 ${device.deviceName} 的访问权限`}
          >
            <Trash2 size={15} /> {revokingDeviceId === device.id ? "撤销中" : "撤销访问权限"}
          </button>
        </div>
      </article>
    {/each}
  </div>
{:else if loading}
  <div class="empty-state compact remote-device-empty-state" role="status">
    <CircleHelp size={22} />
    <strong>正在读取已配对设备</strong>
    <p>Remote Host 就绪后会在这里显示客户端。</p>
  </div>
{:else if unavailable}
  <div class="empty-state compact remote-device-empty-state" role="status">
    <CircleHelp size={22} />
    <strong>暂时无法读取设备</strong>
    <p>请查看上方 Remote Host 状态并重试。</p>
  </div>
{:else}
  <div class="empty-state remote-device-empty-state">
    <Smartphone size={22} />
    <strong>还没有已配对设备</strong>
    <p>添加设备后，Remote 客户端会显示在这里。</p>
  </div>
{/if}
