<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { RefreshCw, Settings } from '@lucide/svelte';
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
  let render: AbortController | undefined;
  async function refresh() {
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

<section class="knowledge-page" aria-label="知识图谱">
  <div class="graph-toolbar">
    <div class="graph-summary"><strong>任务知识图谱</strong><span>{loaded && nodes ? `${nodes} 个节点 · ${links} 条关系` : '查看已完成执行加工后的实体与关系'}</span></div>
    <div class="graph-actions"><Button class="graph-button" type="button" onclick={onModels}><Settings size={15} />模型配置</Button><Button class="graph-button" type="button" onclick={refresh} disabled={busy}><RefreshCw size={15} />{busy ? '读取中…' : '刷新图谱'}</Button></div>
  </div>
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if processing}<p class="notice" role="status">后台正在初始化或加工任务记录，原有预览会保留。请稍后刷新。</p>{/if}
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
  .knowledge-page{display:flex;flex-direction:column;min-width:0;min-height:560px;height:calc(100dvh - 180px);position:relative;gap:12px;color:var(--app-text)}
  .graph-toolbar{display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap;flex:none}.graph-summary{display:grid;gap:6px;min-width:0}.graph-summary strong{font:var(--font-section)}.graph-summary span{font-size:var(--font-size-sm);color:var(--app-muted)}
  .graph-actions{display:flex;gap:8px;flex-wrap:wrap}.graph-surface{flex:1;min-width:0;min-height:460px;position:relative;border:1px solid var(--app-border);border-radius:12px;overflow:hidden;background:var(--app-surface)}
  .knowledge-page :global(.graph-button){display:inline-flex;align-items:center;justify-content:center;gap:7px;min-height:34px;padding:7px 12px;border:1px solid var(--app-border);border-radius:8px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--font-size-sm);cursor:pointer}.knowledge-page :global(.graph-button:disabled){opacity:.6;cursor:wait}.knowledge-page :global(.graph-button:focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}
  .graph-surface :global(iframe){position:absolute;inset:0;width:100%;height:100%}.graph-surface.empty{display:none}.notice{margin:0;padding:10px 12px;border:1px solid var(--app-border);border-radius:8px;color:var(--app-muted);font-size:var(--font-size-sm)}.error{color:var(--color-main-danger-text)}
  .empty-state{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;text-align:center;min-height:320px;padding:24px;border:1px solid var(--app-border);border-radius:12px;background:var(--app-surface)}.empty-state h3{font:var(--font-section);margin:0}.empty-state p{margin:0;max-width:420px;color:var(--app-muted);font-size:var(--font-size-sm);line-height:var(--line-height-body-relaxed)}
  @media(max-height:760px){.knowledge-page{min-height:400px}.graph-surface{min-height:350px}}
</style>
