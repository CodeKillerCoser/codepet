<script lang="ts">
  import * as Dialog from "../ui/dialog/index";
  import { Button } from "../ui/button/index";
  import type { DirtyConversation, ExtractionJob, LineageThread } from "../taskLineage";
  export let open = false;
  export let sourceName: string;
  export let jobs: ExtractionJob[];
  export let dirty: DirtyConversation[];
  export let threads: LineageThread[];
  const jobLabel = (state: string) => ({ queued: "排队中", running: "抽取中", completed: "已完成", failed: "失败", interrupted: "已中断" })[state] ?? state;
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="history-drawer" overlayClass="history-overlay" showCloseButton={false} interactOutsideBehavior="ignore">
    <header class="history-head">
      <Dialog.Title id="extraction-history-title">抽取历史</Dialog.Title>
      <Button aria-label="关闭抽取历史" onclick={() => open = false}>关闭</Button>
    </header>
    <Dialog.Description class="sr-only">查看来源的任务抽取运行结果。</Dialog.Description>
    <div class="history-content">
      <p class="hint">{sourceName} · {jobs.length} 次运行 · {dirty.filter(s => s.state === "dirty").length} 个会话待整理</p>
      {#if !jobs.length}<p class="empty">暂无抽取记录。手动与自动运行的结果会显示在这里。</p>{/if}
      {#each jobs as job (job.id)}
        <article class="history-job">
          <div><strong>{jobLabel(job.state)}</strong><span>{job.id.startsWith("scheduled-") ? "自动" : "手动"}</span></div>
          <p>{threads.find(t => t.id === job.threadId)?.title ?? (job.threadId ? "对话记录不可用" : "全部近期历史")}</p>
          <time>{new Date(job.createdAt).toLocaleString()}</time>
          {#if job.finishedAt}<p>耗时 {Math.max(0, Math.round((job.finishedAt - job.createdAt) / 1000))} 秒</p>{/if}
          {#if job.error}<p class="error">{job.error}</p>{/if}
        </article>
      {/each}
    </div>
  </Dialog.Content>
</Dialog.Root>

<style>
  :global(.history-drawer) { inset: 0 0 0 auto; transform: none; margin: 0; width: min(440px,100vw); max-width: 100vw; height: 100dvh; max-height: 100dvh; box-sizing: border-box; border: 0; border-left: 1px solid var(--app-border); padding: 0; background: var(--app-surface); color: var(--app-text); box-shadow: -12px 0 40px var(--color-history-drawer-shadow); }
  :global(.history-drawer[data-state="open"]) { display: flex; flex-direction: column; }
  .history-head { display: flex; justify-content: space-between; align-items: center; gap: 16px; padding: 24px; border-bottom: 1px solid var(--app-border); }
  .history-head :global(h3) { margin: 0; font-size: var(--font-size-md-plus); overflow-wrap: anywhere; }
  .history-head :global(button) { border: 1px solid var(--app-border); border-radius: 6px; padding: 6px 10px; background: var(--app-surface); color: var(--app-text); font-size: 12px; }
  .history-content { padding: 24px; overflow: auto; min-height: 0; }
  .history-job { border: 1px solid var(--app-border); border-radius: 12px; padding: 16px; margin-top: 12px; font-size: 12px; overflow-wrap: anywhere; }
  .history-job > div { display: flex; justify-content: space-between; gap: 12px; }
  .history-job time, .history-job span, .hint, .empty { color: var(--app-muted); }
  .hint { font-size: 11px; margin: 6px 0 12px; overflow-wrap: anywhere; }
  .empty { font-size: 12px; padding: 16px; }
  .error { color: var(--color-main-danger-text); overflow-wrap: anywhere; }
</style>
