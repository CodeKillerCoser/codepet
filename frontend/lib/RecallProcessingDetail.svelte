<script lang="ts">
  import * as Dialog from './ui/dialog/index';
  import { Button } from './ui/button/index';
  import { X } from '@lucide/svelte';
  import HorizontalTabs from './HorizontalTabs.svelte';
  import RecallStatusIcon from './RecallStatusIcon.svelte';
  import { completedTime, duration, historyRequest, processingLabels, tokenLabel, type ProcessingJob } from './recallHistory';
  export let job: ProcessingJob | null = null;
  export let onClose: () => void;
  let detail: ProcessingJob | null = null;
  let loading = false, error = '', tab: 'result' | 'runtime' = 'result';
  let generation = 0;
  $: open = !!job;
  $: if (job) void load(job.id); else { generation++; detail = null; }
  $: current = detail ?? job;
  async function load(id: string) {
    const request = ++generation;
    detail = null; loading = true; error = ''; tab = 'result';
    try {
      const result = await historyRequest<ProcessingJob>('detail', {jobId: id});
      if (request === generation) detail = result;
    } catch (cause) { if (request === generation) error = String(cause); }
    finally { if (request === generation) loading = false; }
  }
</script>

<Dialog.Root {open} onOpenChange={(value) => { if (!value) onClose(); }}>
  <Dialog.Content class="recall-detail-dialog" showCloseButton={false}>
    <header class="detail-heading"><Dialog.Title>加工详情</Dialog.Title><Button variant="ghost" size="icon-sm" aria-label="关闭加工详情" onclick={onClose}><X size={18}/></Button></header>
    <Dialog.Description class="sr-only">具体任务执行的加工结果、实际模型用量与运行信息。</Dialog.Description>
    {#if current}
      <div class="detail-scroll">
        <div class="detail-title"><h3>{current.title}</h3><span><RecallStatusIcon state={current.state}/>{processingLabels[current.state]}</span></div>
        <p class="muted identity">{current.taskTitle} · Run {current.runId.slice(0, 8)}</p>
        <p class="muted metadata">{completedTime(current.finishedAt)} · {duration(current.elapsedMs)} · {tokenLabel(current)}</p>
        <div class="detail-tabs"><HorizontalTabs items={[{route: 'result', label: '加工结果'}, {route: 'runtime', label: '运行信息'}]} value={tab} label="加工详情分区" panelId="recall-detail-panel" onChange={(value) => tab = value}/></div>
        {#if loading}<p role="status" class="muted">读取加工详情…</p>{/if}
        {#if error}<p role="alert" class="error">{error}</p><Button variant="outline" size="sm" onclick={() => job && load(job.id)}>重新读取</Button>{/if}
        {#if tab === 'result'}
          <div tabindex="0" id="recall-detail-panel" role="tabpanel" aria-labelledby="tab-result">
            <h4>任务摘要</h4>
            {#each current.result?.summaries ?? [] as summary}<p class="summary">{summary}</p>{:else}<p class="muted">{current.state === 'completed' ? '此记录没有保存摘要快照。' : current.error ?? current.phase ?? '尚未产生加工结果。'}</p>{/each}
            {#if current.result}
              <div class="section-heading"><h4>关联实体</h4><span class="muted">{current.result.entityCount ?? current.result.entities.length} 个关联实体</span></div>
              {#if current.result.entities.length}
                <details class="entities"><summary>查看原生实体</summary><ul>{#each current.result.entities as entity (entity.id)}<li><strong>{entity.name}</strong><span class="muted">{entity.type}</span>{#if entity.description}<p>{entity.description}</p>{/if}</li>{/each}</ul></details>
              {:else}<p class="muted">没有可展示的实体快照。</p>{/if}
              {#if current.result.note}<p class="muted">{current.result.note}</p>{/if}
              {#if current.result.truncated}<p class="muted">结果较多，此处展示部分原生结果。</p>{/if}
              <p class="index-result">{current.result.indexUpdated ? '索引已更新' : '索引尚未更新'}{current.result.snapshotUnavailable ? ' · 结果快照暂不可用' : ''}</p>
            {/if}
            <h4>Token 用量</h4>
            {#if current.usageTracked && current.usage?.models.length}
              <div class="usage-table"><table><thead><tr><th>模型用途</th><th>输入</th><th>输出</th><th>合计</th></tr></thead><tbody>{#each current.usage.models as usage}<tr><td>{usage.purpose === 'llm' ? '语言模型' : '向量模型'}</td><td>{usage.reportedRequests ? usage.inputTokens.toLocaleString() : '未报告'}</td><td>{usage.purpose === 'embedding' ? '—' : usage.reportedRequests ? usage.outputTokens.toLocaleString() : '未报告'}</td><td>{usage.reportedRequests ? (usage.inputTokens + usage.outputTokens).toLocaleString() : '未报告'}{usage.reportedRequests > 0 && usage.reportedRequests < usage.requests ? '（部分）' : ''}</td></tr>{/each}</tbody></table></div>
            {:else}<p class="muted">{!current.usageTracked ? '此记录未关联请求用量，无法追溯 Token。' : current.usage?.requests ? '此回执仅包含汇总用量，分模型明细未提供。' : '尚无模型调用记录。'}</p>{/if}
            <p class="muted">仅统计上游返回的实际用量；未返回或不完整的用量标记为“未报告”或“部分”。</p>
          </div>
        {:else}
          <div tabindex="0" id="recall-detail-panel" role="tabpanel" aria-labelledby="tab-runtime"><dl>
            <dt>加工 ID</dt><dd>{current.id}</dd><dt>任务执行</dt><dd>{current.runId}</dd>
            <dt>加入队列</dt><dd>{completedTime(current.queuedAt)}</dd><dt>开始时间</dt><dd>{completedTime(current.startedAt)}</dd>
            <dt>完成时间</dt><dd>{completedTime(current.finishedAt)}</dd><dt>语言模型</dt><dd>{current.models.llm || '未记录'}</dd>
            <dt>向量模型</dt><dd>{current.models.embedding || '未记录'}</dd><dt>文档版本</dt><dd>{current.contentHash}</dd>
            <dt>工作区</dt><dd>{current.workspace || '未记录'}</dd>
          </dl></div>
        {/if}
        <details class="processing-log"><summary>加工日志</summary><dl><dt>当前阶段</dt><dd>{current.phase || '未记录'}</dd><dt>结果</dt><dd>{current.resultSummary || '暂无'}</dd>{#if current.error}<dt>错误类型</dt><dd>{current.error}</dd>{/if}</dl><p class="muted">不记录模型提示词、API 密钥或原始响应日志。</p></details>
      </div>
    {/if}
    <footer><Button variant="outline" size="sm" onclick={onClose}>关闭</Button></footer>
  </Dialog.Content>
</Dialog.Root>

<style>
  :global(.cn-dialog-content.recall-detail-dialog){width:min(720px,calc(100vw - 24px));max-height:calc(100dvh - 32px);display:flex;flex-direction:column;overflow:hidden;padding:0;gap:0;border:1px solid var(--app-border);border-radius:10px;color:var(--app-text);background:var(--app-surface)}
  :global(.recall-detail-dialog button:not([role=tab])){display:inline-flex;align-items:center;justify-content:center;min-height:30px;padding:5px 10px;border:0;border-radius:6px;background:transparent;color:var(--app-text);font:inherit;cursor:pointer}:global(.recall-detail-dialog button:not([role=tab]):hover){background:var(--color-main-hover)}:global(.recall-detail-dialog button:focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}:global(.recall-detail-dialog footer button){border:1px solid var(--app-border)}
  .detail-heading{display:flex;justify-content:space-between;align-items:center;padding:16px 24px;border-bottom:1px solid var(--app-border);flex:none}.detail-heading :global([data-slot=dialog-title]){margin:0;font:var(--font-section)}
  .detail-scroll{padding:20px 24px;overflow:auto;min-height:0;overflow-wrap:anywhere;font-size:var(--font-size-md);line-height:var(--line-height-body-relaxed)}
  .detail-title,.section-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.detail-title h3{margin:0;font:var(--font-section)}.detail-title>span{display:flex;gap:6px;align-items:center;white-space:nowrap;font-size:var(--font-size-sm)}
  .muted{color:var(--app-muted);font-size:var(--font-size-sm)}p{margin:10px 0}.identity{margin-bottom:4px}.metadata{margin-top:0}.detail-tabs{margin:18px 0}.detail-tabs :global(.horizontal-tabs){margin-bottom:0}
  h4{font:var(--font-section);margin:20px 0 10px}[role=tabpanel]>h4:first-child{margin-top:0}.summary{white-space:pre-wrap}.section-heading{margin-top:20px;border-top:1px solid var(--app-border)}
  .entities summary,.processing-log summary{cursor:pointer;padding:8px 0}.entities ul{padding:0;list-style:none}.entities li{padding:10px 0;border-bottom:1px solid var(--app-border)}.entities li>span{margin-left:10px}.entities li p{margin:6px 0}.entities strong{font-weight:var(--font-weight-medium)}
  .index-result{color:var(--app-good);font-size:var(--font-size-sm)}.usage-table{overflow-x:auto}table{border-collapse:collapse;width:100%;font-size:var(--font-size-sm)}th,td{text-align:right;padding:8px 10px;border-bottom:1px solid var(--app-border);white-space:nowrap}th{font-weight:var(--font-weight-medium);background:var(--app-surface-soft)}th:first-child,td:first-child{text-align:left}
  dl{display:grid;grid-template-columns:88px minmax(0,1fr);gap:10px;font-size:var(--font-size-sm)}dt{color:var(--app-muted)}dd{margin:0;overflow-wrap:anywhere}.processing-log{margin-top:20px;border-top:1px solid var(--app-border)}footer{display:flex;justify-content:flex-end;padding:14px 24px;border-top:1px solid var(--app-border);flex:none}.error{color:var(--app-danger)}
  [role=tabpanel]:focus-visible,summary:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:3px}
  @media(max-width:600px){.detail-heading,.detail-scroll,footer{padding-left:16px;padding-right:16px}.detail-title{align-items:flex-start}.metadata{line-height:var(--line-height-body)}}
</style>
