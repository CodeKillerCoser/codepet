<script lang="ts">
  import { onMount } from "svelte";
  import { ExternalLink, FolderOpen } from "@lucide/svelte";
  import SettingsRow from "./SettingsRow.svelte";
  import { getSystemPermissionSettings, openFullDiskAccessSettings, revealPermissionApp, type SystemPermissionSettings } from "./permissions";

  let permissions: SystemPermissionSettings | null = null;
  let loading = true;
  let busy = false;
  let error = "";
  let message = "";

  async function load() {
    loading = true; error = "";
    try { permissions = await getSystemPermissionSettings(); }
    catch { error = "无法读取系统权限设置，请确认已安装最新版本的 Code Pet。"; }
    finally { loading = false; }
  }
  onMount(() => { void load(); });

  async function perform(action: () => Promise<void>, success: string) {
    if (busy) return;
    busy = true; error = ""; message = "";
    try { await action(); message = success; }
    catch (cause) { error = String(cause); }
    finally { busy = false; }
  }
</script>

<div class="settings-workspace permission-settings">
  <section class="settings-section" aria-labelledby="file-access-heading">
    <h3 id="file-access-heading">文件访问</h3>
    <div class="settings-card">
      {#if loading}
        <p role="status">正在读取系统权限设置…</p>
      {:else if permissions?.fullDiskAccessSupported}
        <SettingsRow label="完全磁盘访问权限" description="允许 Code Pet 及其启动的 Agent 访问受 macOS 保护的文件。需要跨目录处理项目，或远程操作因文件访问受阻时，可在此授权。">
          <button type="button" disabled={busy} on:click={() => perform(openFullDiskAccessSettings, "已打开系统设置，请在那里开启 Code Pet 的权限。")}>前往授权 <ExternalLink size={15} /></button>
        </SettingsRow>
        <div class="permission-guide">
          <p>此权限覆盖范围较广。只需访问文稿或下载目录时，也可保留按目录授权。</p>
          <ol>
            <li>打开“隐私与安全性 → 完全磁盘访问权限”。</li>
            <li>开启 Code Pet。若列表中没有它，点击“＋”添加当前运行的 App，或将它从访达拖入列表。</li>
            <li>授权后退出并重新打开 Code Pet，再重试文件操作。</li>
          </ol>
          {#if permissions.appPath}
            <div class="permission-app"><span class="path">{permissions.appPath}</span><button type="button" disabled={busy} on:click={() => perform(revealPermissionApp, "已在访达中显示当前 App，可将它添加到权限列表。")}><FolderOpen size={15} /> 在访达中显示 App</button></div>
          {:else}
            <p>当前不是已打包的 App。请安装并打开 Code Pet.app 后添加权限。</p>
          {/if}
          <p class="permission-note">授权状态以系统设置为准；打开设置不会自动授予权限。</p>
        </div>
      {:else if permissions}
        <p>当前系统没有 macOS 的“完全磁盘访问权限”设置。文件访问由系统及 Agent 自身的权限控制。</p>
      {/if}
      {#if error}<p class="permission-feedback" role="alert">{error} {#if !permissions}<button type="button" on:click={load}>重试</button>{/if}</p>{/if}
      {#if message}<p class="permission-feedback" role="status">{message}</p>{/if}
    </div>
  </section>
</div>

<style>
  .permission-guide { border-top: 1px solid var(--app-border); padding-top: 16px; color: var(--app-muted); font-size: 13px; line-height: 1.7; }
  .permission-guide p { margin: 0 0 12px; }
  .permission-guide ol { margin: 0 0 18px; padding-left: 22px; }
  .permission-guide li + li { margin-top: 8px; }
  .permission-app { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 18px; }
  .permission-app .path { overflow-wrap: anywhere; min-width: 0; flex: 1 1 220px; }
  button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
  .permission-note { font-size: 12px; }
  .permission-feedback { margin: 12px 0; font-size: 13px; overflow-wrap: anywhere; }
</style>
