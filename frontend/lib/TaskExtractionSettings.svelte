<script lang="ts">
  import { onMount } from "svelte";
  import { lineageApi, type ExtractionSettings, type LineageOptions } from "./taskLineage";
  export let providerId: string;
  export let options: LineageOptions;
  export let onSaved: (config: ExtractionSettings) => void;
  let config: ExtractionSettings | null = null, skills: string[] = [], preview = "", error = "", saving = false, saved = false;
  $: instance = options.instances?.find(i => i.id === config?.harnessInstanceId) ?? (!config?.harnessInstanceId && options.instances?.length === 1 ? options.instances[0] : undefined);
  $: models = instance?.controls?.modelCatalog?.models ?? [];
  $: efforts = instance?.controls?.reasoningEffort?.options ?? [];
  $: supported = !!config && !instance?.error && models.some(m => m.id === config?.model && m.enabled !== false) && efforts.some(e => e.id === config?.reasoningEffort && e.enabled !== false);
  async function load() {
    try { [config, skills] = await Promise.all([lineageApi.settings(providerId), lineageApi.skills(providerId)]); await showSkill(); }
    catch (e) { error = String(e); }
  }
  async function showSkill() { if (!config) return; try { preview = (await lineageApi.skill(providerId, config.skill)).text; } catch (e) { error = String(e); } }
  async function save() {
    if (!config || saving) return; saving = true; error = ""; saved = false;
    try { config = await lineageApi.saveSettings(providerId, config); onSaved(config); saved = true; }
    catch (e) { error = String(e); } finally { saving = false; }
  }
  onMount(load);
</script>

<section aria-label="任务抽取设置" class="settings">
  <h3>任务抽取 Agent</h3>
  {#if config}
    <form on:submit|preventDefault={save} on:input={() => saved = false}>
      <div class="fields">
        <label>Harness<select bind:value={config.harness}><option value="claude">Claude Provider</option></select></label>
        <label>运行实例<select bind:value={config.harnessInstanceId}><option value={null}>自动选择唯一 Provider 实例</option>{#each options.instances ?? [] as instance}<option value={instance.id}>{instance.name}</option>{/each}</select></label>
        <label>模型<select aria-label="模型" bind:value={config.model} required>{#if !models.some(m => m.id === config?.model)}<option value={config.model} disabled>{config.model} · 当前实例未提供</option>{/if}{#each models as model}<option value={model.id} disabled={model.enabled === false}>{model.displayName}</option>{/each}</select></label>
        <label>推理强度<select aria-label="推理强度" bind:value={config.reasoningEffort}>{#if !efforts.some(e => e.id === config?.reasoningEffort)}<option value={config.reasoningEffort} disabled>{config.reasoningEffort} · 当前实例未提供</option>{/if}{#each efforts as effort}<option value={effort.id} disabled={effort.enabled === false}>{effort.displayName}</option>{/each}</select></label>
        <label>抽取 Skill<select aria-label="抽取 Skill" bind:value={config.skill} on:change={showSkill}>{#each skills as skill}<option value={skill}>{skill}</option>{/each}</select></label>
      </div>
      <label>提取任务的提示词<textarea bind:value={config.prompt} maxlength="8000" rows="4" placeholder="例如：将同一验收目标的实现与验证归入一个任务。"></textarea></label>
      <details><summary>查看当前 Skill</summary><pre>{preview}</pre></details>
      <h3 class="schedule-heading">定时任务</h3>
      <label class="check"><input type="checkbox" bind:checked={config.automatic} />自动提取</label>
      <label>自动提取间隔（秒）<input type="number" min="15" max="86400" bind:value={config.intervalSeconds} required /></label>
      <p>开启后，在应用运行期间按间隔持续整理当前来源最近 48 小时的新正文，失败时暂停。任务页的手动整理使用同一 Agent。</p>
      <details><summary>高级执行选项与工作目录</summary><div class="fields">
        <label>超时（秒）<input type="number" min="15" max="300" bind:value={config.timeoutSeconds} required /></label>
        <label>消息静默等待（秒）<input type="number" min="0" max="3600" bind:value={config.debounceSeconds} required /></label>
      </div><p>Skill 目录：{options.layout?.skills}<br />抽取工作区：{options.layout?.extractionWorkspaces}<br />任务工作区：{options.layout?.taskWorkspaces}</p></details>
      <p>模型和推理选项来自所选 Provider。每次运行使用独立抽取目录，继承 Provider 的本机配置；配置从下一次运行生效。当前 Provider 不提供单批美元硬预算，运行由分批和超时控制。</p>
      {#if !supported}<p role="alert">{instance?.error ?? "请选择可用的 Provider 实例及其支持的模型、推理强度；连接变更后请刷新来源。"}</p>{/if}
      <button type="submit" disabled={saving || (config.automatic && !supported)}>{saving ? "保存中…" : "保存 Agent 设置"}</button>
      <button type="button" on:click={load} disabled={saving}>重新加载配置和 Skill</button>
      {#if saved}<span role="status">已保存</span>{/if}
    </form>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>

<style>
  .schedule-heading{margin-top:22px}.check{flex-direction:row;align-items:center}.check input{width:auto}details{margin:12px 0}details .fields{margin-top:12px}
  .settings{padding:16px;margin:8px 0 16px;border:1px solid var(--app-border);border-radius:8px;background:var(--app-surface);font-size:12px;min-width:0}h3{margin:0 0 12px;font-size:14px}.fields{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(190px,100%),1fr));gap:12px}label{display:flex;flex-direction:column;gap:5px;min-width:0;margin-bottom:10px}input,select,textarea{width:100%;min-width:0;color:var(--app-text);background:var(--app-bg);border:1px solid var(--app-border);border-radius:5px;padding:7px;box-sizing:border-box}textarea{resize:vertical}p{color:var(--app-muted);overflow-wrap:anywhere;line-height:1.6}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto}button{padding:7px 10px;margin:0 8px 6px 0;border:1px solid var(--app-border);border-radius:5px;color:var(--app-text);background:var(--app-bg)}:is(button,input,select,textarea,summary):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}[role=alert]{color:var(--color-main-danger-text)}span{color:var(--app-muted)}
</style>
