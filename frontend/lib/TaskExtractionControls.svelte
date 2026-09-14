<script lang="ts">
  import { onMount } from "svelte";
  import { lineageApi, type LineageOptions, type LineageSnapshot, type ExtractionJob } from "./taskLineage";
  export let providerId: string;
  export let active = true;
  export let options: LineageOptions;
  let data: LineageSnapshot = { threads: [], tasks: [], pendingMessages: 0, diagnostics: [] };
  let jobs: ExtractionJob[] = [], scope = "", busy = "", error = "", disposed = false;
  $: extracting = jobs.some(job => ["queued", "running"].includes(job.state));
  async function refresh() {
    const [snapshot, history] = await Promise.all([lineageApi.snapshot(providerId), lineageApi.jobs(providerId)]);
    if (disposed) return;
    data = snapshot; jobs = history;
    if (scope && !data.threads.some(thread => thread.id === scope)) scope = "";
  }
  async function run(label: string, action: () => Promise<unknown>) {
    if (busy) return;
    busy = label; error = "";
    try { await action(); await refresh(); } catch (e) { error = String(e); } finally { busy = ""; }
  }
  onMount(() => {
    void run("读取记录", async () => {});
    const timer = setInterval(() => { if (active && !busy) void refresh().catch(e => error = String(e)); }, 3000);
    return () => { disposed = true; clearInterval(timer); };
  });
</script>

<h3>记录与手动抽取</h3>
<div class="settings-card">
  <div class="row"><div>本地记录<p>扫描本地 Agent 的会话，更新任务列表中的对话树。</p></div><button disabled={!!busy} on:click={() => run("扫描中…", () => lineageApi.scan(providerId))}>扫描记录</button></div>
  <div class="row"><label for="extraction-scope">整理范围</label><select id="extraction-scope" bind:value={scope} disabled={!!busy || extracting}><option value="">全部近期历史</option>{#each data.threads as thread}<option value={thread.id}>{thread.title}</option>{/each}</select></div>
  <div class="row"><div>手动抽取<p role="status">{data.pendingMessages} 条近 48 小时消息待分析</p></div><button disabled={!!busy || extracting || !options.instances?.some(i => i.controls && !i.error)} on:click={() => run("提交整理…", () => lineageApi.trigger(providerId, scope || null))}>{extracting ? "整理中…" : "手动整理历史"}</button></div>
  {#if data.lastExtraction}<div class="row"><span>最近实际模型</span><span>{Object.keys(data.lastExtraction.modelUsage ?? {}).join("、") || "Provider 未返回"}</span></div>{/if}
</div>
<p>仅抽取最近 48 小时的用户消息与 AI 正文，排除工具执行、推理及无可靠时间戳的内容。每次处理一批近期正文并保留进度；自动提取在应用运行期间按设置间隔继续处理。由所选 Provider 执行，当前不提供美元硬预算。</p>
{#if error}<p role="alert">{error}</p>{/if}
{#if data.diagnostics.length}<details><summary>{data.diagnostics.length} 项记录读取或抽取问题</summary>{#each data.diagnostics as diagnostic}<p>{diagnostic}</p>{/each}</details>{/if}

<style>
  h3{font-size:14px;margin:24px 0 16px}.row{display:flex;align-items:center;justify-content:space-between;gap:20px;padding:18px 0;font-size:12px}.row+.row{border-top:1px solid var(--app-border)}.row>div{min-width:0}.row>span:last-child{text-align:right;overflow-wrap:anywhere}p{font-size:12px;line-height:1.6;color:var(--app-muted);overflow-wrap:anywhere;margin:6px 0}.row p{margin-bottom:0}button,select{min-height:34px;padding:6px 10px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-surface);color:var(--app-text);font-size:12px}button{flex-shrink:0}select{max-width:60%;min-width:0}button:disabled{opacity:.5}details{font-size:12px;margin-top:12px}[role=alert]{color:var(--color-main-danger-text)}:is(button,select):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}
</style>
