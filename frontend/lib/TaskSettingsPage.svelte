<script lang="ts">
  import { onMount } from "svelte";
  import TaskExtractionSettings from "./TaskExtractionSettings.svelte";
  import { lineageApi, type LineageOptions, type LineageThread, type LineageWatch } from "./taskLineage";
  let options: LineageOptions | null = null, providerId = "", error = "", loading = true, saving = false;
  let threads: LineageThread[] = [], watch: LineageWatch | null = null, saved = false;
  async function sourceChanged() {
    loading = true; error = ""; saved = false; watch = null;
    try { const snapshot = await lineageApi.snapshot(providerId); threads = snapshot.threads; watch = structuredClone(options?.sources.find(s => s.id === providerId)?.watch ?? { revision: 0, enabled: false, extract: false, threadId: null, model: "haiku", remainingJobs: 5, lastError: null }); }
    catch (e) { error = String(e); } finally { loading = false; }
  }
  async function load() {
    loading = true; error = "";
    try { options = await lineageApi.options(); if (!options.sources.some(s => s.id === providerId)) providerId = options.sources[0]?.id ?? ""; if (providerId) await sourceChanged(); }
    catch (e) { error = String(e); } finally { loading = false; }
  }
  async function saveSchedule() {
    if (!watch || saving) return; saving = true; error = ""; saved = false;
    try {
      const config = await lineageApi.settings(providerId);
      watch = await lineageApi.watch(providerId, { ...watch, extract: watch.enabled && watch.extract, model: config.model });
      if (options) options = { ...options, sources: options.sources.map(s => s.id === providerId ? { ...s, watch: watch! } : s) };
      saved = true;
    } catch (e) { error = String(e); } finally { saving = false; }
  }
  onMount(load);
</script>

<div class="settings-page">
  <section id="task-extraction-section" aria-labelledby="task-extraction-heading">
    <header><h2 id="task-extraction-heading">任务抽取</h2><p>管理任务分析使用的数据来源、AI 能力和后台执行方式。</p></header>
    <div class="source"><label>Codex 数据来源<select aria-label="Codex 数据来源" bind:value={providerId} on:change={sourceChanged} disabled={loading || saving}>{#each options?.sources ?? [] as source}<option value={source.id}>{source.name}</option>{/each}</select></label><button on:click={load} disabled={loading || saving}>刷新来源</button></div>
    {#if loading}<p role="status">加载设置…</p>{:else if options && providerId}
      {#key providerId}<TaskExtractionSettings {providerId} {options} onSaved={() => {}} />{/key}
      {#if watch}
        <section class="schedule" aria-label="后台执行">
          <h3>后台执行</h3>
          <form on:submit|preventDefault={saveSchedule} on:input={() => saved = false}>
            <label class="check"><input type="checkbox" bind:checked={watch.enabled} />后台更新</label>
            <label class="check"><input type="checkbox" bind:checked={watch.extract} disabled={!watch.enabled} />定时抽取</label>
            <div class="fields"><label>抽取范围<select aria-label="抽取范围" bind:value={watch.threadId}><option value={null}>选择对话</option>{#each threads as thread}<option value={thread.id}>{thread.title}</option>{/each}</select></label><label>允许抽取批次<input type="number" min="0" max="5" required bind:value={watch.remainingJobs} /></label></div>
            <p>选中主对话时包含派生线程。每次最多允许 5 批，失败暂停；扫描和静默间隔使用上方配置。仅在应用运行期间执行。</p>
            {#if !threads.length}<p>暂无会话，请先在任务页扫描记录。</p>{/if}
            {#if watch.lastError}<p role="alert">{watch.lastError}</p>{/if}
            <button type="submit" disabled={saving || (watch.enabled && watch.extract && (!watch.threadId || !options.claudeExecutable || watch.remainingJobs === 0))}>保存后台设置</button>
            {#if saved}<span role="status">后台设置已保存</span>{/if}
          </form>
        </section>
      {/if}
      <p class="boundary">抽取对象固定为最近 48 小时的用户消息与 AI 正文，工具执行与推理内容不参与抽取。</p>
    {:else}<p>请先在“连接”中配置 Codex 来源，再设置任务抽取。</p>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
  </section>
</div>

<style>
  .settings-page{padding:20px 24px;overflow:auto;min-width:0;color:var(--app-text);font-family:var(--font-family-ui)}h2{font-size:18px;margin:0 0 6px}h3{font-size:14px;margin:0 0 12px}p{font-size:12px;color:var(--app-muted);line-height:1.6;overflow-wrap:anywhere}.source{display:flex;gap:12px;align-items:end;margin:18px 0}.source label{flex:1;max-width:360px}label{display:flex;flex-direction:column;gap:6px;font-size:12px;min-width:0}.fields{display:grid;grid-template-columns:minmax(0,2fr) minmax(0,1fr);gap:12px;margin-top:14px}select,input:not([type=checkbox]){width:100%;min-width:0;box-sizing:border-box;min-height:34px;padding:6px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-bg);color:var(--app-text)}button{padding:7px 10px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-surface);color:var(--app-text);font-size:12px}.schedule{padding:16px;border:1px solid var(--app-border);border-radius:8px}.check{display:inline-flex;flex-direction:row;align-items:center;margin-right:20px}.boundary,span{font-size:12px;color:var(--app-muted)}span{margin-left:10px}[role=alert]{color:var(--color-main-danger-text)}:is(button,input,select):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}@media(max-width:650px){.settings-page{padding:16px}.fields{grid-template-columns:1fr}.source{flex-wrap:wrap}}
</style>
