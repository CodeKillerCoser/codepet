<script lang="ts">
  import { onMount } from "svelte";
  import { modelGatewayRequest, type ModelSettings, type ModelUsage } from "./modelGateway";
  let settings: ModelSettings | null = null;
  let usage: ModelUsage[] = [];
  let busy = false, loading = true, error = "", message = "";
  let extraBody = "{}";
  async function load() {
    loading = true; error = "";
    try {
      const result = await modelGatewayRequest("model-settings");
      settings = result.settings; usage = result.usage.models;
      extraBody = JSON.stringify(settings.llmExtraBody, null, 2);
    } catch (cause) { error = String(cause); }
    finally { loading = false; }
  }
  async function save() {
    if (busy || !settings) return;
    busy = true; error = ""; message = "";
    try {
      const parsed = JSON.parse(extraBody);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("高级参数需要 JSON 对象");
      const result = await modelGatewayRequest("save-model-settings", {...settings, llmExtraBody: parsed});
      settings = result.settings; usage = result.usage.models;
      message = settings.enabled ? "已保存，后台将加工已完成的任务执行记录。" : "已停用语义与图加工，全文搜索继续可用。";
    } catch (cause) { error = String(cause); }
    finally { busy = false; }
  }
  onMount(load);
</script>

<section class="settings-workspace model-settings" aria-label="模型配置">
  <p>配置用于任务召回的模型。支持本机 MLX 服务与兼容 OpenAI 的云端 API，模型文件由服务端管理。</p>
  <p>使用云端 API 时，已完成执行的任务内容、进度和验证证据会发送给配置的模型服务。</p>
  <p>API Key 留空保留当前值；更换 API 地址时请重新填写。</p>
  {#if loading}<p role="status">加载模型配置…</p>{:else if settings}
    <form on:submit|preventDefault={save}>
      <fieldset disabled={busy}>
        <legend>任务召回</legend>
        <div class="settings-card"><label>启用语义与图召回<input type="checkbox" bind:checked={settings.enabled} /></label></div>
        {#each ["llm", "embedding"] as purpose}
          {@const profile = purpose === "llm" ? settings.llm : settings.embedding}
          <h3>{purpose === "llm" ? "LLM · 文档与实体加工" : "Embedding · 向量索引"}</h3>
          <div class="settings-card">
            <label>API 地址<input aria-label={`${purpose} API 地址`} type="url" placeholder="http://127.0.0.1:11234/v1" bind:value={profile.endpoint} required={settings.enabled} autocomplete="off" spellcheck="false" /></label>
            <label>模型名称<input aria-label={`${purpose} 模型名称`} bind:value={profile.model} required={settings.enabled} autocomplete="off" spellcheck="false" /></label>
            <label>API Key<input aria-label={`${purpose} API Key`} type="password" bind:value={profile.apiKey} placeholder={profile.hasApiKey ? "已保存，留空保留" : "本机服务可留空"} autocomplete="new-password" /></label>
            {#if purpose === "embedding"}<label>向量维度<input type="number" min="1" max="65536" bind:value={profile.dimensions} required={settings.enabled} /></label>{/if}
          </div>
        {/each}
        <details><summary>模型兼容选项</summary>
          <label class="option"><input type="checkbox" bind:checked={settings.plainJson} />关闭 json_object 强制格式（部分本机服务更快）</label>
          <label class="advanced">LLM 附加 JSON 参数<textarea bind:value={extraBody} rows="4" spellcheck="false"></textarea></label>
          <p>支持 chat_template_kwargs、reasoning_effort 和 thinking。使用 MLX Qwen 时可填写 {JSON.stringify({chat_template_kwargs:{enable_thinking:false}})}。</p>
        </details>
        <div class="actions"><button type="submit">{busy ? "保存中…" : "保存模型配置"}</button></div>
      </fieldset>
    </form>
    {#if message}<p role="status">{message}</p>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
    <section aria-label="模型调用用量"><div class="usage-heading"><h3>模型调用用量</h3><button type="button" on:click={load} disabled={busy}>刷新</button></div>
      <p>累计统计来自 CodePet Gateway 的请求。Token 采用上游报告值，未报告的请求单独计数。</p>
      {#each usage as row}<div class="usage-row"><strong>{row.model}</strong><span>{row.purpose === "llm" ? "LLM" : "Embedding"} · {row.requests} 次请求 · {row.requests - row.succeeded} 次失败</span><span>输入 {row.inputTokens.toLocaleString()} / 输出 {row.outputTokens.toLocaleString()} tokens · {row.requests - row.reportedRequests} 次未报告</span></div>{:else}<p>暂无模型调用。</p>{/each}
    </section>
  {:else}<button type="button" on:click={load}>重新加载</button>{#if error}<p role="alert">{error}</p>{/if}{/if}
</section>

<style>
  .model-settings{min-width:0;color:var(--app-text);font-family:var(--font-family-ui)}
  fieldset{min-width:0;border:0;padding:0;margin:0} legend{font:var(--font-section);margin-bottom:16px}
  h3{font:var(--font-section);margin:24px 0 16px} p{font-size:var(--font-size-sm);color:var(--app-muted);line-height:var(--line-height-body-relaxed);overflow-wrap:anywhere}
  input,textarea{min-width:0;padding:8px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-bg);color:var(--app-text);font:inherit}
  details{margin:20px 0} summary{cursor:pointer} .option{display:flex;gap:10px;align-items:center;margin:16px 0}.advanced{display:grid;gap:10px}.advanced textarea{width:100%;box-sizing:border-box}
  button{padding:8px 12px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-surface);color:var(--app-text);font:inherit;cursor:pointer}button:disabled{opacity:.6;cursor:wait}
  .actions{display:flex;justify-content:flex-end;margin:16px 0}.usage-heading{display:flex;justify-content:space-between;align-items:center;gap:12px}.usage-row{display:grid;gap:6px;border-top:1px solid var(--app-border);padding:14px 0;overflow-wrap:anywhere}.usage-row span{font-size:var(--font-size-sm);color:var(--app-muted)}
  [role=alert]{color:var(--color-main-danger-text)} :is(input,textarea,button,summary):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}
  @media(max-width:600px){.model-settings :global(.settings-card label){align-items:flex-start;flex-direction:column}.model-settings :global(.settings-card label > input:not([type=checkbox])){width:100%;max-width:100%;box-sizing:border-box;margin:0}}
</style>
