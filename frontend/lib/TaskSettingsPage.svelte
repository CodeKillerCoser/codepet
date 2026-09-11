<script lang="ts">
  import { onMount } from "svelte";
  import TaskExtractionSettings from "./TaskExtractionSettings.svelte";
  import { lineageApi, type LineageOptions } from "./taskLineage";
  let options: LineageOptions | null = null, providerId = "", error = "", loading = true;
  async function load() {
    loading = true; error = "";
    try { options = await lineageApi.options(); if (!options.sources.some(s => s.id === providerId)) providerId = options.sources[0]?.id ?? ""; }
    catch (e) { error = String(e); } finally { loading = false; }
  }
  onMount(load);
</script>

<section class="settings-page" aria-labelledby="task-extraction-heading">
  <header><h2 id="task-extraction-heading">任务抽取</h2><p>配置负责整理对话历史的 Agent，以及它的自动运行间隔。</p></header>
  <div class="source"><label>本地 Agent<select aria-label="本地 Agent" bind:value={providerId} disabled={loading}>{#each options?.sources ?? [] as source}<option value={source.id}>{source.name}</option>{/each}</select></label><button on:click={load} disabled={loading}>刷新</button></div>
  {#if options?.sources.find(s => s.id === providerId)?.directory}<p>记录目录：{options.sources.find(s => s.id === providerId)?.directory}</p>{/if}
  {#if options?.sources.find(s => s.id === providerId)?.error}<p role="alert">{options.sources.find(s => s.id === providerId)?.error}</p>{/if}
  <p>自动读取本地 Agent 的会话记录，无需配置数据来源连接。目前支持 Codex。</p>
  {#if loading}<p role="status">加载设置…</p>{:else if options && providerId}
    {#key providerId}<TaskExtractionSettings {providerId} {options} onSaved={() => { if (options) options = { ...options, sources: options.sources.map(s => s.id === providerId ? {...s, watch: s.watch ? {...s.watch, lastError: null} : undefined} : s) }; }} />{/key}
    {#if options.sources.find(s => s.id === providerId)?.watch?.lastError}<p role="alert">自动提取已暂停：{options.sources.find(s => s.id === providerId)?.watch?.lastError}</p>{/if}
    {#if !options.instances?.some(i => i.controls && !i.error)}<p>请在“连接”中配置并启动 Claude Provider，再刷新来源。</p>{/if}
    <p>仅整理最近 48 小时的用户消息与 AI 正文，工具执行与推理内容不参与抽取。任务页可以查看任务图、整理历史和手动运行。</p>
  {:else}<p>暂未发现支持的本地 Agent。</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>

<style>
  .settings-page{padding:20px 24px;min-width:0;color:var(--app-text);font-family:var(--font-family-ui)}.settings-page header h2{font-size:20px;margin:0 0 8px}p{font-size:12px;color:var(--app-muted);line-height:1.6;overflow-wrap:anywhere}.source{display:flex;gap:12px;align-items:end;margin:18px 0}.source label{display:flex;flex-direction:column;gap:6px;flex:1;max-width:360px;min-width:0;font-size:12px}select{min-width:0;width:100%;min-height:34px;padding:6px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-bg);color:var(--app-text)}button{padding:7px 10px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-surface);color:var(--app-text);font-size:12px}[role=alert]{color:var(--color-main-danger-text)}:is(button,select):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}@media(max-width:650px){.settings-page{padding:16px}.source{flex-wrap:wrap}}
</style>
