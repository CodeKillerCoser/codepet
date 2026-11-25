<script lang="ts">
  import { onMount, tick } from "svelte";
  import LocalModelEnvironment from "./LocalModelEnvironment.svelte";
  import ModelProfileEditor from "./ModelProfileEditor.svelte";
  import { sameEndpoint, isLocalEndpoint } from "./modelProviders";
  import {
    modelGatewayRequest,
    type ModelSettings,
    type ModelUsage,
  } from "./modelGateway";
  import "./modelSettings.css";
  let settings: ModelSettings | null = null,
    saved: ModelSettings | null = null;
  let usage: ModelUsage[] = [],
    loading = true,
    busy = false,
    error = "",
    message = "",
    extraBody = "{}",
    revision = 0;
  let form: HTMLFormElement,
    embeddingEditor: ModelProfileEditor,
    saveButton: HTMLButtonElement;
  async function load() {
    loading = true;
    error = "";
    try {
      const result = await modelGatewayRequest("model-settings");
      settings = structuredClone(result.settings);
      saved = structuredClone(result.settings);
      usage = result.usage.models;
      extraBody = JSON.stringify(settings.llmExtraBody, null, 2);
      revision++;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  }
  async function save() {
    if (busy || !settings) return;
    error = "";
    message = "";
    if (
      settings.enabled &&
      (!Number.isInteger(settings.embedding.dimensions) ||
        settings.embedding.dimensions < 1 ||
        settings.embedding.dimensions > 65536)
    ) {
      embeddingEditor.showAdvanced();
      await tick();
    }
    if (settings.enabled && !form.reportValidity()) return;
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(extraBody);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
        throw Error("高级参数需要 JSON 对象");
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      return;
    }
    busy = true;
    try {
      const result = await modelGatewayRequest("save-model-settings", {
        ...settings,
        llmExtraBody: parsed,
      });
      settings = structuredClone(result.settings);
      saved = structuredClone(result.settings);
      usage = result.usage.models;
      revision++;
      message = settings.enabled
        ? "模型配置已保存，已完成的任务执行将进入加工队列。"
        : "已保存，任务语义与图加工已停用。";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
      await tick();
      saveButton?.focus({ preventScroll: true });
    }
  }
  function useLocal(endpoint: string) {
    if (!settings) return;
    const apply = (profile: ModelSettings["llm"]) =>
      sameEndpoint(profile.endpoint, endpoint)
        ? profile
        : {
            ...profile,
            endpoint,
            model: "",
            apiKey: "",
            hasApiKey: false,
            dimensions: 0,
          };
    settings = {
      ...settings,
      llm: apply(settings.llm),
      embedding: apply(settings.embedding),
    };
    revision++;
    message = "已填入本地 API 地址，请选择两个模型后保存。";
  }
  async function refreshUsage() {
    try {
      usage = (await modelGatewayRequest("model-settings")).usage.models;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }
  onMount(load);
</script>

<section class="settings-workspace model-settings" aria-label="模型配置">
  <LocalModelEnvironment onUseLocal={useLocal} />
  <section class="model-config" aria-label="模型配置表单">
    <div class="model-section-heading">
      <h3>模型配置</h3>
      <span>连接本地或云端的 OpenAI 兼容 API</span>
    </div>
    {#if loading}<p class="model-hint" role="status">加载模型配置…</p>
    {:else if settings && saved}
      <form bind:this={form} novalidate on:submit|preventDefault={save}>
        <fieldset disabled={busy}>
          <label class="model-enable">
            <span>启用任务语义与图加工</span><input
              type="checkbox"
              bind:checked={settings.enabled}
            />
          </label>
          {#if !isLocalEndpoint(settings.llm.endpoint) || !isLocalEndpoint(settings.embedding.endpoint)}<p
              class="model-hint"
            >
              任务内容会发送到你配置的模型服务。
            </p>{/if}
          {#snippet languageAdvanced()}
            <details class="model-advanced">
              <summary>高级设置<span>输出格式与附加参数</span></summary>
              {#if settings}<label class="model-check">
                  <input
                    type="checkbox"
                    bind:checked={settings.plainJson}
                  />使用普通 JSON 输出兼容模式
                </label>{/if}
              <p class="model-hint">
                仅在本地模型或服务不支持 json_object 格式约束时启用。
              </p>
              <label class="model-field model-json">
                <span>附加参数</span><textarea
                  aria-label="语言模型附加参数"
                  bind:value={extraBody}
                  rows="4"
                  spellcheck="false"></textarea>
              </label>
              <p class="model-hint">
                支持 thinking、reasoning_effort、chat_template_kwargs。
              </p>
            </details>
          {/snippet}
          {#key revision}
            <ModelProfileEditor
              purpose="llm"
              bind:profile={settings.llm}
              savedProfile={saved.llm}
              enabled={settings.enabled}
              advanced={languageAdvanced}
            />
            <ModelProfileEditor
              purpose="embedding"
              bind:profile={settings.embedding}
              savedProfile={saved.embedding}
              enabled={settings.enabled}
              bind:this={embeddingEditor}
            />
          {/key}
          <div class="model-save-row">
            <span class="model-hint">
              密钥留空保留已保存值；更换地址后需重新填写。
            </span><button type="submit" bind:this={saveButton}>
              {busy ? "保存中…" : "保存模型配置"}
            </button>
          </div>
        </fieldset>
      </form>
      {#if message}<p class="model-message" role="status">{message}</p>{/if}
      <details class="model-usage">
        <summary>模型调用用量</summary>
        <div class="model-section-heading">
          <p class="model-hint">按上游报告的实际 Token 统计</p>
          <button type="button" disabled={busy} on:click={refreshUsage}>
            刷新用量
          </button>
        </div>
        {#each usage as row}<div class="model-usage-row">
            <strong>{row.model}</strong><span>
              {row.purpose === "llm" ? "语言模型" : "向量模型"} · {row.requests} 次请求
              · {row.requests - row.succeeded} 次失败
            </span><span>
              输入 {row.inputTokens.toLocaleString()} / 输出 {row.outputTokens.toLocaleString()}
              Token · {row.requests - row.reportedRequests} 次未报告
            </span>
          </div>{:else}<p class="model-hint">暂无模型调用。</p>{/each}
      </details>
    {:else}<button type="button" on:click={load}>重新加载模型配置</button>{/if}
    {#if error}<p class="model-field-error" role="alert">{error}</p>{/if}
  </section>
</section>
