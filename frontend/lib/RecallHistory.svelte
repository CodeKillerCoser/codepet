<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ArrowUpRight, RefreshCw } from '@lucide/svelte';
  import { Button } from './ui/button/index';
  import AppSidePanel from './shell/AppSidePanel.svelte';
  import RecallStatusIcon from './RecallStatusIcon.svelte';
  import RecallProcessingDetail from './RecallProcessingDetail.svelte';
  import { completedTime, duration, historyRequest, resultLabel, tokenLabel, type ProcessingHistory, type ProcessingJob } from './recallHistory';
  export let open = false;
  export let returnFocus: HTMLElement | undefined = undefined;
  export let onClose: () => void;
  export let onProcessed: () => void = () => {};
  let history: ProcessingHistory | null = null;
  let error = '', loading = false, busy = '', filter = '', page = 0;
  let selected: ProcessingJob | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0, disposed = false;
  let completed = new Set<string>();
  $: if (open) { filter; page; void load(false); } else closeHistory();
  function closeHistory() { generation++; clearTimeout(timer); loading = false; selected = null; }
  async function load(quiet: boolean) {
    clearTimeout(timer);
    const request = ++generation;
    if (!quiet) loading = true;
    try {
      const value = await historyRequest<ProcessingHistory>('list', {offset: page * 40, limit: 40, ...(filter ? {state: filter} : {})});
      if (disposed || !open || request !== generation) return;
      const next = new Set(value.items.filter(job => job.state === 'completed').map(job => job.id));
      if (history && [...next].some(id => !completed.has(id))) onProcessed();
      completed = next; history = value; error = '';
    } catch (cause) { if (!disposed && open && request === generation) error = String(cause); }
    finally {
      if (!disposed && open && request === generation) {
        loading = false;
        timer = setTimeout(() => void load(true), 5000);
      }
    }
  }
  async function act(job: ProcessingJob, action: 'cancel' | 'retry') {
    if (busy) return;
    busy = job.id; error = '';
    // Invalidate an already running poll before changing the queue.
    generation++; clearTimeout(timer);
    try { await historyRequest(action, {jobId: job.id}); await load(true); }
    catch (cause) { if (!disposed && open) { await load(true); error = String(cause); } }
    finally { busy = ''; }
  }
  onDestroy(() => { disposed = true; generation++; clearTimeout(timer); });
</script>

<AppSidePanel {open} title="加工历史" description="任务执行的加工队列、结果与模型用量" closeLabel="关闭加工历史" {returnFocus} {onClose}>
  <div class="recall-history">
    <div class="history-controls">
      <select aria-label="筛选加工记录" bind:value={filter} onchange={() => page = 0}>
        <option value="">全部记录</option><option value="queued">排队中</option><option value="processing">加工中</option><option value="completed">已完成</option><option value="failed">失败</option><option value="cancelled">已取消</option><option value="interrupted">已中断</option><option value="paused">已暂停</option><option value="superseded">版本已更新</option>
      </select>
      <div class="history-counts" aria-label="加工状态汇总">{#each ['completed', 'failed', 'queued', 'processing'] as state}<span><RecallStatusIcon state={state as ProcessingJob['state']}/><span>{history?.counts[state as ProcessingJob['state']] ?? 0}</span></span>{/each}</div>
      <Button variant="ghost" size="icon-sm" aria-label="刷新加工历史" title="刷新加工历史" disabled={loading || !!busy} onclick={() => load(false)}><RefreshCw size={15}/></Button>
    </div>
    {#if loading}<p class="muted" role="status">读取加工历史…</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if history}
      <div class="history-list">{#each history.items as job (job.id)}
        <article class="history-job" aria-label={job.title}>
          <div class="job-heading"><h4 title={job.title}>{job.title}</h4><RecallStatusIcon state={job.state}/><Button variant="ghost" size="icon-xs" aria-label={`查看加工详情：${job.title}`} title="弹出加工详情" onclick={() => selected = job}><ArrowUpRight size={16}/></Button></div>
          <p class="job-identity" title={`${job.taskTitle} · ${job.runId}`}>{job.taskTitle} · Run {job.runId.slice(0, 8)}</p>
          <div class="job-result"><p title={resultLabel(job)}>{resultLabel(job)}</p>{#if job.canCancel}<Button variant="ghost" size="xs" disabled={!!busy} onclick={() => act(job, 'cancel')}>{busy === job.id ? '取消中…' : '取消排队'}</Button>{:else if job.canRetry}<Button variant="ghost" size="xs" disabled={!!busy} onclick={() => act(job, 'retry')}>{busy === job.id ? '提交中…' : '重试'}</Button>{/if}</div>
          <p class="job-metadata">{completedTime(job.finishedAt)} · {job.state === 'processing' ? '已用 ' : ''}{duration(job.elapsedMs)} · {tokenLabel(job)}</p>
        </article>
      {:else}<p class="empty">{filter ? '没有符合条件的加工记录。' : '暂无加工记录。已完成的任务执行进入队列后会显示在这里。'}</p>{/each}</div>
      {#if page > 0 || history.hasMore}<div class="pagination"><Button variant="ghost" size="sm" disabled={page === 0 || loading} onclick={() => page--}>上一页</Button><span class="muted">第 {page + 1} 页</span><Button variant="ghost" size="sm" disabled={!history.hasMore || loading} onclick={() => page++}>下一页</Button></div>{/if}
    {/if}
  </div>
</AppSidePanel>
<RecallProcessingDetail job={selected} onClose={() => selected = null}/>

<style>
  .recall-history{min-width:0;color:var(--app-text);font-size:var(--font-size-sm)}.history-controls{display:flex;align-items:center;gap:8px;margin-bottom:14px;flex-wrap:wrap}
  .recall-history :global(button){display:inline-flex;align-items:center;justify-content:center;gap:5px;min-height:28px;border:0;border-radius:6px;background:transparent;color:var(--app-text);font:inherit;padding:4px 7px;cursor:pointer;box-sizing:border-box}.recall-history :global(button:hover){background:var(--color-main-hover)}.recall-history :global(button:focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}.recall-history :global(button:disabled){opacity:.5;cursor:wait}.job-heading :global(button),.history-controls :global(button){width:28px;padding:4px}
  select{font:inherit;color:var(--app-text);background:var(--app-surface);border:1px solid var(--app-border);border-radius:6px;padding:6px 8px;min-width:0;max-width:100%}.history-counts{display:flex;gap:10px;align-items:center;margin-left:auto}.history-counts>span{display:inline-flex;align-items:center;gap:4px;color:var(--app-muted);font-variant-numeric:tabular-nums}
  .history-list{display:grid;gap:10px}.history-job{border:1px solid var(--app-border);border-radius:var(--radius-md,8px);padding:12px;min-width:0;background:var(--app-surface)}
  .job-heading{display:flex;align-items:center;gap:7px;min-width:0}.job-heading h4{font:var(--font-section);font-size:var(--font-size-md);margin:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.job-heading :global(button){margin-left:auto;flex:none}
  .job-identity,.job-metadata{color:var(--app-muted);font-size:var(--font-size-sm);margin:5px 0 0;line-height:var(--line-height-body-compact)}.job-identity{overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.job-metadata{overflow-wrap:anywhere;font-variant-numeric:tabular-nums}
  .job-result{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-top:6px;min-width:0;min-height:26px}.job-result p{margin:0;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;line-height:var(--line-height-body)}.job-result :global(button){flex:none;color:var(--app-muted)}
  .muted,.empty{color:var(--app-muted)}.empty{line-height:var(--line-height-body);margin:24px 0}.error{color:var(--app-danger);overflow-wrap:anywhere;line-height:var(--line-height-body)}.pagination{display:flex;justify-content:space-between;align-items:center;gap:8px;margin-top:16px}select:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}
</style>
