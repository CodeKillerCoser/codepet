<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { frontendBuild, webcontentInfo, loadLatestWebcontent, needsPageReload, prepareReload, consumeReloadNotice, reloadPage, extractionDraftDirty, webcontentReloading } from "./webcontent";
  import logo from "../../src-tauri/icons/icon.png";
  import SettingsRow from "./SettingsRow.svelte";
  export let checking = false;
  export let message = "";
  export let error = "";
  export let ignoredVersion: string | null = null;
  export let onCheck: () => void;
  export let reloadBlocked = false;
  export let reloaded = false;
  export let onExtraction: () => void;
  let appVersion = "读取中…";
  let canReload = false, availability = "正在读取加载能力…", reloadError = "", reloadMessage = reloaded ? "已重新加载前端资源。" : "";
  onMount(() => {
    getVersion().then(value => appVersion = value).catch(() => appVersion = "暂时无法读取");
    webcontentInfo().then(info => { canReload = info.canReload; availability = info.canReload ? "" : "开发模式由 Vite 自动更新前端资源。"; })
      .catch(() => availability = "当前 App 尚不支持热加载，请先更新原生程序。");
  });
  async function loadLatest() {
    if (!canReload || $webcontentReloading || reloadBlocked) return;
    reloadError = ""; reloadMessage = "";
    if ($extractionDraftDirty) { reloadError = "任务抽取有未保存配置，请先保存或重新加载配置后再试。"; return; }
    webcontentReloading.set(true);
    try {
      prepareReload();
      const next = await loadLatestWebcontent();
      if (needsPageReload(frontendBuild, next)) {
        reloadPage(next);
      } else {
        consumeReloadNotice();
        reloadMessage = "当前已是最新前端资源。";
        webcontentReloading.set(false);
      }
    } catch (error) { consumeReloadNotice(); reloadError = String(error); webcontentReloading.set(false); }
  }
</script>

<div class="settings-workspace">
  <div class="about-identity"><img src={logo} alt="Code Pet Logo" width="80" height="80" /><h2>Code Pet</h2></div>
  <section class="settings-section"><h3>版本信息</h3><div class="settings-card">
    <SettingsRow label="App 版本"><span>{appVersion}</span></SettingsRow>
    <SettingsRow label="前端资源版本" description={`构建于 ${new Date(frontendBuild.builtAt).toLocaleString()}`}>
      <span>{frontendBuild.version}</span>
      <button type="button" disabled={!canReload || reloadBlocked || $webcontentReloading} on:click={loadLatest}>加载最新</button>
    </SettingsRow>
    {#if availability}<p class="settings-update-message">{availability}</p>{/if}
    {#if reloadBlocked}<p class="settings-update-message" role="status">请等待当前设置保存或操作完成后再加载。</p>{/if}
    {#if reloadError}<p class="settings-update-message" role="alert">{reloadError} {#if $extractionDraftDirty}<button on:click={onExtraction}>前往任务抽取</button>{/if}</p>{/if}
    {#if reloadMessage}<p class="settings-update-message" role="status">{reloadMessage}</p>{/if}
  </div></section>
  <section class="settings-section"><h3>应用更新</h3><div class="settings-card">
    <SettingsRow label="检查更新" description={ignoredVersion ? `已忽略版本 ${ignoredVersion}` : "检查是否有可用的新版本"}>
      <button type="button" disabled={checking} on:click={onCheck}>{checking ? "检查中…" : "检查更新"}</button>
    </SettingsRow>
    {#if error || message}<p class="settings-update-message" role={error ? "alert" : "status"}>{error || message}</p>{/if}
  </div></section>
</div>

<style>
  .about-identity{display:flex;flex-direction:column;align-items:center;gap:12px;padding:20px 0}.about-identity img{object-fit:contain}.about-identity h2{margin:0;font-size:22px}
</style>
