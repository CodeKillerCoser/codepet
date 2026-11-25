<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Button } from './ui/button/index';
  import { loadRecallGraph, renderRecallGraph, type GraphView } from './cogneeGraph';
  export let onModels: () => void = () => {};
  let host: HTMLDivElement;
  let view: GraphView | undefined;
  let busy = false, loaded = false, error = '';
  let processing = false;
  let nodes = 0, links = 0;
  let rendering = false;
  let disposed = false;
  export let revision = 0;
  let renderedRevision = 0;
  $: if (revision !== renderedRevision && loaded && !busy) { renderedRevision = revision; void refresh(); }
  let render: AbortController | undefined;
  export async function refresh() {
    if (busy) return;
    busy = true; error = ''; processing = false;
    try {
      const graph = await loadRecallGraph();
      if (disposed) return;
      if (!Array.isArray(graph.nodes) || !Array.isArray(graph.links)) throw new Error('图数据格式无效，请重试');
      if (!graph.nodes.length) {
        view?.destroy(); view = undefined; nodes = links = 0; loaded = true;
        return;
      }
      // Keep the existing view until a new native renderer has initialized.
      rendering = true;
      await tick();
      render = new AbortController();
      const next = await renderRecallGraph(host, graph, render.signal);
      if (disposed) { next.destroy(); return; }
      view?.destroy(); view = next;
      nodes = graph.nodes.length; links = graph.links.length; loaded = true;
    } catch (cause) {
      if (!disposed) {
        processing = String(cause).includes('(409)');
        if (!processing) error = String(cause);
      }
    } finally { if (!disposed) { busy = false; rendering = false; } }
  }
  onMount(() => { void refresh(); return () => { disposed = true; render?.abort(); view?.destroy(); }; });
</script>

<section class="knowledge-page" aria-label="任务图谱" tabindex="-1">
  {#if error}<div class="notice error" role="alert">{error}{#if loaded}<Button class="graph-button" type="button" disabled={busy} onclick={refresh}>重新读取</Button>{/if}</div>{/if}
  {#if processing}<div class="notice" role="status">后台正在初始化或加工任务记录，原有预览会保留。{#if loaded}<Button class="graph-button" type="button" disabled={busy} onclick={refresh}>刷新图谱</Button>{/if}</div>{/if}
  {#if busy}<p class="notice" role="status">{nodes ? '正在更新图谱，当前预览保持可用…' : '正在读取真实任务图谱…'}</p>{/if}
  <div class="graph-surface" class:empty={!nodes && !rendering} bind:this={host} aria-label="Cognee 原生图预览"></div>
  {#if loaded && !nodes}
    <div class="empty-state"><h3>还没有可预览的知识图谱</h3><p>在模型配置中启用任务召回。已完成的执行加工后，会在这里显示真实图谱。</p><Button class="graph-button" type="button" onclick={onModels}>配置模型</Button></div>
  {:else if !loaded && !busy && processing}
    <div class="empty-state"><h3>图谱正在准备中</h3><p>后台加工完成后，刷新即可查看真实图谱。</p><Button class="graph-button" type="button" onclick={refresh}>刷新图谱</Button></div>
  {:else if !loaded && !busy && error}
    <div class="empty-state"><h3>暂时无法读取图谱</h3><p>检查模型配置与后台服务，然后刷新重试。</p><Button class="graph-button" type="button" onclick={refresh}>重新读取</Button></div>
  {/if}
</section>

<style>
  .knowledge-page{display:flex;flex-direction:column;min-width:0;min-height:0;height:100%;flex:1;position:relative;gap:0;color:var(--app-text)}
  .graph-surface{flex:1;min-width:0;min-height:0;position:relative;overflow:hidden;background:var(--app-surface)}
  .knowledge-page :global(.graph-button){display:inline-flex;align-items:center;justify-content:center;gap:7px;min-height:34px;padding:7px 12px;border:1px solid var(--app-border);border-radius:8px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--font-size-sm);cursor:pointer}.knowledge-page :global(.graph-button:disabled){opacity:.6;cursor:wait}.knowledge-page :global(.graph-button:focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}
  .graph-surface :global(iframe){position:absolute;inset:0;width:100%;height:100%}.graph-surface.empty{display:none}.notice{margin:0;padding:10px 12px;border:1px solid var(--app-border);border-radius:8px;color:var(--app-muted);font-size:var(--font-size-sm);display:flex;align-items:center;justify-content:space-between;gap:12px}.error{color:var(--color-main-danger-text)}
  .empty-state{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;text-align:center;min-height:320px;padding:24px;border:1px solid var(--app-border);border-radius:12px;background:var(--app-surface)}.empty-state h3{font:var(--font-section);margin:0}.empty-state p{margin:0;max-width:420px;color:var(--app-muted);font-size:var(--font-size-sm);line-height:var(--line-height-body-relaxed)}
</style>
