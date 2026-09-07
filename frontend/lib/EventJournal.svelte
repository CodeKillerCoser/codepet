<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  type Entry = { sequence: number; receivedAt: number; source: string; provider: string; payload: unknown };
  type HistoryCursor = { segmentId: string; beforeOffset: number };
  type Page = { events: Entry[]; cursor: number; nextBefore: number | null; nextHistoryCursor: HistoryCursor | null; resetReason: string | null; hasMore: boolean; resetRequired: boolean; dropped: number; error: string | null };
  let entries: Entry[] = [];
  let expanded = new Set<number>();
  let keyword = "";
  let source = "";
  let paused = false;
  let loading = false;
  let ready = false;
  let error = "";
  let retryMode: "initial" | "live" | "older" = "initial";
  let storageError = "";
  let dropped = 0;
  let notice = "";
  let cursor = 0;
  let before: number | null = null;
  let historyCursor: HistoryCursor | null = null;
  let hasMore = false;
  let revision = 0;
  let disposed = false;
  let debounce: ReturnType<typeof setTimeout>;

  async function fetchPage(mode: "initial" | "live" | "older") {
    if (disposed || loading || (mode !== "initial" && !ready)) return;
    const requestRevision = revision;
    loading = true;
    let reset = false;
    try {
      const page = await invoke<Page>("query_event_journal", { query: {
        keyword, source, limit: 100,
        ...(mode === "live" ? { after: cursor } : mode === "older" ? historyCursor ? { historyCursor } : { before } : {}),
      } });
      if (disposed || requestRevision !== revision) return;
      error = "";
      storageError = page.error ?? "";
      dropped = page.dropped;
      if (page.resetRequired) {
        notice = page.resetReason === "history_expired"
          ? "历史分片已被轮转清理，列表将重新加载现存日志。"
          : "实时缓存已过期，列表将从最近日志恢复；更早记录可分页查看。";
        reset = true;
      } else if (mode === "initial") {
        entries = page.events;
        expanded = new Set();
        cursor = page.cursor;
        before = page.nextBefore;
        historyCursor = page.nextHistoryCursor;
        hasMore = page.hasMore;
        ready = true;
      } else if (mode === "older") {
        entries = [...entries, ...page.events.filter(e => !entries.some(current => current.sequence === e.sequence))];
        before = page.nextBefore;
        historyCursor = page.nextHistoryCursor;
        hasMore = page.hasMore;
      } else {
        const merged = new Map(entries.map(e => [e.sequence, e]));
        for (const entry of page.events) merged.set(entry.sequence, entry);
        const all = [...merged.values()].sort((a, b) => b.sequence - a.sequence);
        entries = all.slice(0, 1000);
        expanded = new Set([...expanded].filter(sequence => merged.has(sequence) && entries.some(entry => entry.sequence === sequence)));
        if (all.length > entries.length) {
          hasMore = true;
          historyCursor = null;
          before = entries.at(-1)?.sequence ?? before;
        }
        cursor = page.cursor;
      }
    } catch (cause) {
      if (!disposed && requestRevision === revision) { error = String(cause); retryMode = mode; }
    } finally {
      if (!disposed && requestRevision === revision) {
        loading = false;
        if (reset) void fetchPage("initial");
      }
    }
  }

  function filterChanged() {
    revision += 1;
    loading = false;
    ready = false;
    entries = [];
    expanded = new Set();
    hasMore = false;
    historyCursor = null;
    before = null;
    notice = "";
    clearTimeout(debounce);
    debounce = setTimeout(() => void fetchPage("initial"), 300);
  }

  function loadOlder() {
    paused = true;
    void fetchPage("older");
  }

  onMount(() => {
    void fetchPage("initial");
    const timer = setInterval(() => {
      if (!document.hidden && !paused) void fetchPage(ready ? "live" : "initial");
    }, 1000);
    return () => { disposed = true; clearInterval(timer); clearTimeout(debounce); };
  });
</script>

<section class="journal pixel-panel" aria-label="事件日志">
  <div class="filters">
    <input type="search" aria-label="事件关键词" placeholder="搜索原始 JSON、Provider 或关键词" bind:value={keyword} on:input={filterChanged} />
    <select aria-label="事件来源" bind:value={source} on:change={filterChanged}>
      <option value="">全部来源</option>
      <option value="hook">Hook 原始数据</option>
      <option value="provider">Provider 数据</option>
    </select>
    <button type="button" aria-pressed={paused} on:click={() => { paused = !paused; }}>{paused ? "恢复实时" : "暂停刷新"}</button>
  </div>
  <div class="status" aria-live="polite">
    <span>{entries.length} 条已加载 · {paused ? "已暂停" : "实时刷新"}</span>
    {#if loading}<span>加载中…</span>{/if}
  </div>
  {#if error}<p role="alert">日志加载失败：{error} <button on:click={() => void fetchPage(retryMode)}>重试</button></p>{/if}
  {#if storageError}<p role="alert">最近一次日志写入错误：{storageError}</p>{/if}
  {#if dropped}<p role="status">本次运行有 {dropped} 条记录因队列已满或写入失败而未保存。</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  <div class="records">
    {#each entries as entry (entry.sequence)}
      <details open={expanded.has(entry.sequence)} on:toggle={(event) => {
        const next = new Set(expanded);
        if (event.currentTarget.open) next.add(entry.sequence); else next.delete(entry.sequence);
        expanded = next;
      }}>
        <summary>
          <span class="source">{entry.source === "hook" ? "HOOK" : "PROVIDER"}</span>
          <strong>{entry.provider}</strong>
          <span class="method">{(entry.payload as any)?.method ?? (entry.payload as any)?.hook_event_name ?? (entry.payload as any)?.type ?? "JSON"}</span>
          <time datetime={new Date(entry.receivedAt).toISOString()}>{new Date(entry.receivedAt).toLocaleString()}</time>
        </summary>
        {#if expanded.has(entry.sequence)}<pre>{JSON.stringify(entry.payload, null, 2)}</pre>{/if}
      </details>
    {/each}
  </div>
  {#if ready && !entries.length}<p class="empty">{keyword || source ? "没有匹配的事件" : "还没有事件，收到 Hook 或 Provider 数据后会自动显示。"}</p>{/if}
  {#if hasMore}<button class="older" disabled={loading} on:click={loadOlder}>加载更早记录（暂停实时刷新）</button>{/if}
</section>

<style>
  .journal { padding: 20px; min-width: 0; }
  .filters { display: flex; gap: 10px; flex-wrap: wrap; }
  .filters input { flex: 1 1 260px; min-width: 0; }
  .filters select { max-width: 100%; }
  button { border: 1px solid var(--app-border); border-radius: 8px; padding: 9px 12px; color: var(--app-text); background: var(--app-surface-soft); cursor: pointer; }
  button:disabled { opacity: 0.5; cursor: default; }
  button:focus-visible, summary:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  .status { display: flex; gap: 12px; margin: 14px 0; opacity: 0.7; font-size: 12px; }
  details { border-top: 1px solid var(--app-border); }
  summary { display: flex; align-items: center; gap: 12px; cursor: pointer; padding: 12px 4px; flex-wrap: wrap; }
  summary::before { content: "▸"; }
  details[open] summary::before { content: "▾"; }
  summary strong, .method { overflow-wrap: anywhere; }
  .source { font-size: 10px; letter-spacing: 0.05em; opacity: 0.7; }
  .method { flex: 1; font-size: 12px; }
  time { font-size: 11px; opacity: 0.7; }
  pre { max-height: 480px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; padding: 12px; font-size: 12px; background: var(--app-surface-soft); }
  .older { margin-top: 16px; }
  .empty { padding: 30px 0; text-align: center; opacity: 0.7; }
  @media (max-width: 600px) {
    .journal { padding: 12px; }
    summary { gap: 8px; }
    time { width: 100%; padding-left: 18px; }
    .filters button, .filters select { flex: 1; }
  }
</style>
