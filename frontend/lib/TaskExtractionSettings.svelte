<script lang="ts">
  import { onMount } from "svelte";
  import { lineageApi, type ExtractionSettings, type LineageOptions } from "./taskLineage";
  export let providerId: string;
  export let options: LineageOptions;
  export let onSaved: (config: ExtractionSettings) => void;
  let config: ExtractionSettings | null = null, skills: string[] = [], preview = "", error = "", saving = false, saved = false;
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
  <h3>任务抽取设置</h3>
  {#if config}
    <form on:submit|preventDefault={save} on:input={() => saved = false}>
      <div class="fields">
        <label>Harness<select bind:value={config.harness}><option value="claude">Claude CLI</option></select></label>
        <label>摘要实例<select bind:value={config.harnessInstanceId}><option value={null}>自动选择唯一实例 / 本机默认</option>{#each options.instances ?? [] as instance}<option value={instance.id}>{instance.name}</option>{/each}</select></label>
        <label>模型<input bind:value={config.model} required maxlength="120" placeholder="haiku" /></label>
        <label>抽取 Skill<select aria-label="抽取 Skill" bind:value={config.skill} on:change={showSkill}>{#each skills as skill}<option value={skill}>{skill}</option>{/each}</select></label>
        <label>每批预算（美元）<input type="number" min="0.01" max="5" step="0.01" bind:value={config.budgetUsd} required /></label>
        <label>超时（秒）<input type="number" min="15" max="300" bind:value={config.timeoutSeconds} required /></label>
        <label>扫描间隔（秒）<input type="number" min="15" max="86400" bind:value={config.intervalSeconds} required /></label>
        <label>消息静默等待（秒）<input type="number" min="0" max="3600" bind:value={config.debounceSeconds} required /></label>
      </div>
      <label>补充抽取提示词<textarea bind:value={config.prompt} maxlength="8000" rows="3" placeholder="例如：将同一验收目标的实现与验证归入一个任务。"></textarea></label>
      <details><summary>查看当前 Skill</summary><pre>{preview}</pre></details>
      <p>Skill 目录：{options.layout?.skills ?? "用户数据目录 / skills"}<br />抽取工作区：{options.layout?.extractionWorkspaces ?? "用户数据目录 / workspaces/task-extraction"}<br />任务工作区：{options.layout?.taskWorkspaces ?? "用户数据目录 / workspaces/tasks"}</p>
      <p>配置在下一批抽取时生效。模型别名遵循本机映射；定时抽取仅在应用运行且已开启后台更新时执行。</p>
      <button type="submit" disabled={saving}>{saving ? "保存中…" : "保存抽取设置"}</button>
      <button type="button" on:click={load} disabled={saving}>重新加载配置和 Skill</button>
      {#if saved}<span role="status">已保存</span>{/if}
    </form>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>

<style>
  .settings{padding:16px;margin:8px 0 16px;border:1px solid var(--app-border);border-radius:8px;background:var(--app-surface);font-size:12px;min-width:0}h3{margin:0 0 12px;font-size:14px}.fields{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(190px,100%),1fr));gap:12px}label{display:flex;flex-direction:column;gap:5px;min-width:0;margin-bottom:10px}input,select,textarea{width:100%;min-width:0;color:var(--app-text);background:var(--app-bg);border:1px solid var(--app-border);border-radius:5px;padding:7px;box-sizing:border-box}textarea{resize:vertical}p{color:var(--app-muted);overflow-wrap:anywhere;line-height:1.6}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto}button{padding:7px 10px;margin:0 8px 6px 0;border:1px solid var(--app-border);border-radius:5px;color:var(--app-text);background:var(--app-bg)}:is(button,input,select,textarea,summary):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}[role=alert]{color:var(--color-main-danger-text)}span{color:var(--app-muted)}
</style>
