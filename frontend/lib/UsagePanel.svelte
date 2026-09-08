<script lang="ts">
  import { onMount } from "svelte";
  import { codepetGateway } from "./codepetGateway";
  import { bucketLabels, makeUsageQuery } from "./usageQuery";
  import type { ProviderSummary } from "../../sdk/typescript/codepet-gateway-sdk/src/generated";
  import type { UsageDataset, UsageQuery, UsageQueryResult, UsageTimeBucket, UsageMetric } from "../../sdk/typescript/codepet-agent-sdk/src/generated";

  let sources: Array<{ provider: ProviderSummary; dataset: UsageDataset }> = [];
  let selected = "";
  let days = 7;
  let bucket: UsageTimeBucket = "day";
  let model = "";
  let groupModels = false;
  let result: UsageQueryResult | null = null;
  let activeQuery: UsageQuery | null = null;
  let loading = true;
  let error = "";
  let discoveryErrors: string[] = [];
  let revision = 0;
  let disposed = false;
  const metrics: Array<[UsageMetric, string]> = [["totalTokens", "Token 总量"], ["inputTokens", "输入"], ["outputTokens", "输出"], ["cacheReadTokens", "缓存读取"], ["cacheWriteTokens", "缓存写入"]];
  $: source = sources.find((s) => key(s) === selected);
  $: max = Math.max(1, ...(result?.rows.map((row) => row.values.totalTokens?.value ?? 0) ?? []));
  function key(s: {provider: ProviderSummary; dataset: UsageDataset}) { return JSON.stringify([s.provider.id, s.dataset.id]); }
  function number(value: number | null | undefined) { return value == null ? "—" : value.toLocaleString(); }
  function date(value: string) { return new Date(value).toLocaleString(undefined, { timeZone: "UTC", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }); }

  async function discover() {
    const version = ++revision;
    loading = true; error = ""; result = null;
    try {
      const list = await codepetGateway("provider.list", {});
      const descriptions = await Promise.allSettled(list.providers.map(async (provider) => ({provider, description: await codepetGateway("provider.describe", {providerId: provider.id})})));
      if (disposed || version !== revision) return;
      discoveryErrors = descriptions.flatMap((d, i) => d.status === "rejected" ? [`${list.providers[i].identity.displayName}: ${String(d.reason)}`] : []);
      sources = descriptions.flatMap((d) => d.status === "fulfilled" && d.value.description.capabilities.methods.includes("codepet.usage.query") ? (d.value.description.capabilities.usageDatasets ?? []).map((dataset) => ({provider: d.value.provider, dataset})) : []);
      selected = sources.some((s) => key(s) === selected) ? selected : sources[0] ? key(sources[0]) : "";
      await changeSource();
    } catch (e) { if (!disposed && version === revision) error = String(e); }
    finally { if (!disposed) loading = false; }
  }
  async function changeSource() {
    const next = sources.find((s) => key(s) === selected);
    model = ""; groupModels = false;
    if (!next) return;
    bucket = next.dataset.timeBuckets.includes("halfHour") ? "halfHour" : "day";
    await query(next);
  }
  async function query(next = source, more = false) {
    if (!next) return;
    const version = ++revision;
    loading = true; error = "";
    try {
      const request = more && activeQuery && result?.nextCursor ? {...activeQuery, page: {limit: 200, cursor: result.nextCursor}} : makeUsageQuery(next.dataset, days, bucket, model, groupModels);
      if (!more) {result = null; activeQuery = request;}
      const response = await codepetGateway("codepet.usage.query", {providerId: next.provider.id, query: request});
      if (disposed || version !== revision) return;
      result = more && result ? {...response.result, rows: [...result.rows, ...response.result.rows]} : response.result;
    } catch (e) { if (!disposed && version === revision) error = String(e); }
    finally { if (!disposed && version === revision) loading = false; }
  }
  onMount(() => {void discover(); return () => {disposed = true; revision++;};});
</script>

<div class="usage-workspace">
  <section class="usage-panel pixel-panel">
    <header class="section-head"><h3>Token 用量</h3><button disabled={loading} on:click={() => discover()}>刷新</button></header>
    <form class="usage-controls" on:submit|preventDefault={() => query()}>
      <label>数据来源<select bind:value={selected} disabled={loading || !sources.length} on:change={() => changeSource()}>{#each sources as s}<option value={key(s)}>{s.provider.identity.displayName} · {s.dataset.displayName}</option>{/each}</select></label>
      <label>范围<select bind:value={days} disabled={loading} on:change={() => query()}>{#each [1, 7, 30, 90, 365] as value}<option value={value}>最近 {value} 天</option>{/each}</select></label>
      <label>单位<select bind:value={bucket} disabled={loading} on:change={() => query()}>{#each source?.dataset.timeBuckets ?? [] as value}<option value={value}>{bucketLabels[value]}</option>{/each}</select></label>
      {#if source?.dataset.modelFilter}<label>模型 ID<input bind:value={model} placeholder="全部模型" disabled={loading} /></label><button disabled={loading}>筛选</button>{/if}
      {#if source?.dataset.modelGrouping}<label class="model-toggle"><input type="checkbox" bind:checked={groupModels} disabled={loading} on:change={() => query()} />按模型分组</label>{/if}
    </form>
    <p class="usage-note">时间按 UTC 显示。累计与每日峰值基于整个查询范围，不受分页影响。</p>
    {#if source?.dataset.baseBucketMinutes === 1440}<p class="usage-note">该来源仅支持每日总量，不提供模型和输入、输出、缓存拆分。</p>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
    {#each discoveryErrors as failure}<p role="status">{failure}</p>{/each}
    {#if loading}<p role="status">正在查询用量…</p>{/if}
    {#if !loading && !sources.length && !error}<div class="empty-state"><strong>没有可查询的用量来源</strong><p>请先连接支持用量查询的 Provider。</p></div>{/if}
  </section>
  {#if result}
    <section class="usage-summary-grid" aria-label="Token 用量概览">
      {#each metrics as [metric, label]}<article class="overview-card pixel-panel"><span>{label}</span><strong>{number(result.summaries?.totals?.[metric]?.value)}</strong><p>{result.summaries?.totals?.[metric]?.completeness === "partial" ? "部分记录缺少该指标" : result.summaries?.totals?.[metric]?.value == null ? "暂无可用数据" : "查询范围内已记录用量"}</p></article>{/each}
      <article class="overview-card pixel-panel"><span>每日峰值</span><strong>{number(result.summaries?.peakDaily?.totalTokens)}</strong><p>{result.summaries?.peakDaily?.date ?? "暂无峰值"}</p></article>
    </section>
    {#if result.nativeAccountSummary}<p>账户原生累计 {number(result.nativeAccountSummary.lifetimeTokens)} · 账户原生每日峰值 {number(result.nativeAccountSummary.peakDailyTokens)}（不受上方范围筛选影响）</p>{/if}
    <p class="usage-note">{result.coverage.status === "complete" ? "完整覆盖" : "部分覆盖：仅展示来源已提供的记录，缺失数据不计为 0。"} 更新于 {result.updatedAt ? date(result.updatedAt) : "—"} UTC</p>
    <section class="usage-table pixel-panel" aria-label="用量明细">
      <h3>时间与模型明细</h3>
      {#if !result.rows.length}<div class="empty-state">所选范围内没有已记录的用量。</div>{:else}
        <div class="usage-results"><table><thead><tr><th>时间（UTC）</th><th>模型</th>{#each metrics as [,label]}<th>{label}</th>{/each}</tr></thead><tbody>
          {#each result.rows as row}<tr><td>{row.bucket ? date(row.bucket.from) : "累计"}{row.provisional ? " · 进行中" : ""}</td><td>{activeQuery?.aggregation.groupBy.length ? row.modelId ?? "未知模型" : "全部"}</td>{#each metrics as [metric]}<td>{#if metric === "totalTokens"}<span class="token-bar" style={`--fill:${100 * (row.values.totalTokens?.value ?? 0) / max}%`}>{number(row.values[metric]?.value)}</span>{:else}{number(row.values[metric]?.value)}{/if}</td>{/each}</tr>{/each}
        </tbody></table></div>
        {#if result.nextCursor}<button disabled={loading} on:click={() => query(source, true)}>加载更多</button>{/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .usage-controls { flex-wrap: wrap; align-items: end; }
  .usage-controls label { min-width: 0; max-width: 100%; }
  select, input { max-width: 100%; font: inherit; }
  .usage-note { opacity: .75; font-size: .85rem; overflow-wrap: anywhere; }
  .usage-results { overflow-x: auto; max-width: 100%; }
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { padding: 10px; text-align: left; white-space: nowrap; border-bottom: 1px solid var(--app-border); }
  .token-bar { display: block; background: linear-gradient(90deg, var(--app-surface-soft) var(--fill), transparent var(--fill)); padding: 4px; }
  .model-toggle { display: flex; align-items: center; }
  input:not([type="checkbox"]) { color: var(--app-text); background: var(--app-surface-soft); border: 1px solid var(--app-border); border-radius: 8px; min-height: 38px; padding: 0 10px; }
  .usage-workspace, .usage-table { min-width: 0; }
  @media (max-width: 620px) { .usage-controls { justify-content: stretch; } .usage-controls label { width: 100%; } }
  button:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid currentColor; outline-offset: 3px; }
</style>
