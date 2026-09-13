<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { version as frontendVersion } from "../../package.json";
  import logo from "../../src-tauri/icons/icon.png";
  import SettingsRow from "./SettingsRow.svelte";
  export let checking = false;
  export let message = "";
  export let error = "";
  export let ignoredVersion: string | null = null;
  export let onCheck: () => void;
  let appVersion = "读取中…";
  onMount(() => { getVersion().then(value => appVersion = value).catch(() => appVersion = "暂时无法读取"); });
</script>

<div class="settings-workspace">
  <div class="about-identity"><img src={logo} alt="Code Pet Logo" width="80" height="80" /><h2>Code Pet</h2></div>
  <section class="settings-section"><h3>版本信息</h3><div class="settings-card">
    <SettingsRow label="App 版本"><span>{appVersion}</span></SettingsRow>
    <SettingsRow label="前端资源版本"><span>{frontendVersion}</span></SettingsRow>
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
