<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Button } from './ui/button/index';
  import { loadRecallGraph, renderRecallGraph, type GraphView } from './cogneeGraph';
  import { modelGatewayRequest, type ModelSettings } from './modelGateway';
  export let onModels: () => void = () => {};
  let host: HTMLDivElement;
  let view: GraphView | undefined;
  let busy = false, loaded = false, error = '';
  let processing = false;
  let nodes = 0, links = 0;
  let rendering = false;
  let disposed = false;
  let checking = false;
  let version: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let emptySettings: ModelSettings | null = null;
  $: modelsConfigured = !!emptySettings && [emptySettings.llm, emptySettings.embedding].every(profile => profile.endpoint.trim() && profile.model.trim()) && emptySettings.embedding.dimensions > 0;
  export let revision = 0;
  let renderedRevision = 0;
  $: if (revision !== renderedRevision && loaded && !busy) { renderedRevision = revision; void refresh(); }
  let render: AbortController | undefined;
  export async function refresh() {
    await readGraph(false);
  }
  function schedule() {
    clearTimeout(timer);
    if (!disposed) timer = setTimeout(() => {
      if (document.visibilityState === 'hidden') schedule();
      else void readGraph(true);
    }, 5000);
  }
  async function readGraph(quiet: boolean) {
    if (checking || disposed) return;
    checking = true;
    busy = !quiet || !loaded; error = '';
    try {
      const graph = await loadRecallGraph(false, version);
      if (disposed) return;
      processing = !!graph.previewStatus?.updating;
      if (view && graph.previewVersion && graph.previewVersion === version) return;
      if (graph.unchanged) throw new Error('图谱快照已失效，请重新读取');
      if (!Array.isArray(graph.nodes) || !Array.isArray(graph.links)) throw new Error('图数据格式无效，请重试');
      if (!graph.nodes.length) {
        // An empty index does not imply missing models. Read the same saved
        // settings as the settings page; never infer configuration from nodes.
        const configuration = await modelGatewayRequest('model-settings');
        if (disposed) return;
        emptySettings = configuration.settings;
        view?.destroy(); view = undefined; nodes = links = 0; version = undefined; loaded = true;
        return;
      }
      // Keep the existing view until a new native renderer has initialized.
      rendering = true;
      await tick();
      const state = view?.getState?.();
      if (quiet && state?.interacting) return;
      render = new AbortController();
      const next = state ? await renderRecallGraph(host, graph, render.signal, state) : await renderRecallGraph(host, graph, render.signal);
      if (disposed) { next.destroy(); return; }
      if (quiet && view?.getState && JSON.stringify(state) !== JSON.stringify(view.getState())) { next.destroy(); return; }
      view?.destroy(); view = next;
      version = graph.previewVersion;
      nodes = graph.nodes.length; links = graph.links.length; loaded = true;
    } catch (cause) {
      if (!disposed) {
        if (String(cause).includes('(409)')) processing = true;
        else error = String(cause);
      }
    } finally { if (!disposed) { checking = busy = rendering = false; schedule(); } }
  }
  onMount(() => {
    const visible = () => { if (document.visibilityState === 'visible') void readGraph(true); };
    document.addEventListener('visibilitychange', visible);
    void refresh();
    return () => { disposed = true; clearTimeout(timer); document.removeEventListener('visibilitychange', visible); render?.abort(); view?.destroy(); };
  });
</script>

<section class="knowledge-page" aria-label="工作记忆" tabindex="-1">
  {#if error}<div class="notice error" role="alert">{error}{#if loaded}<Button class="graph-button" type="button" disabled={busy} onclick={refresh}>重新读取</Button>{/if}</div>{/if}
  {#if processing}<div class="notice" role="status">后台正在加工任务记录，已完成的图谱保持可用并自动更新。{#if loaded}<Button class="graph-button" type="button" disabled={busy} onclick={refresh}>刷新图谱</Button>{/if}</div>{/if}
  {#if busy}<p class="notice" role="status">{nodes ? '正在更新图谱，当前预览保持可用…' : '正在读取真实任务图谱…'}</p>{/if}
  <div class="graph-surface" class:empty={!nodes && !rendering} bind:this={host} aria-label="Cognee 原生图预览"></div>
  {#if loaded && !nodes && !busy && !error && !processing}
    <div class="empty-state">
      {#if !modelsConfigured}
        <h3>尚未配置任务模型</h3><p>配置语言模型和向量模型，并启用任务加工后，已完成的任务执行会生成图谱。</p><Button class="graph-button" type="button" onclick={onModels}>配置模型</Button>
      {:else if !emptySettings?.enabled}
        <h3>任务加工尚未启用</h3><p>模型已配置。请在模型设置中启用任务加工，已完成的任务执行加工后会生成图谱。</p><Button class="graph-button" type="button" onclick={onModels}>模型设置</Button>
      {:else}
        <h3>还没有可预览的知识图谱</h3><p>任务加工已启用。已完成的任务执行加工后会显示在这里，可在加工历史中查看进度。</p><Button class="graph-button" type="button" onclick={refresh}>刷新图谱</Button>
      {/if}
    </div>
  {:else if !loaded && !busy && processing}
    <div class="empty-state"><h3>正在准备首份图谱</h3><p>第一份图谱就绪后会自动显示，其余任务会继续在后台加工。</p><Button class="graph-button" type="button" onclick={refresh}>刷新图谱</Button></div>
  {:else if !loaded && !busy && error}
    <div class="empty-state"><h3>暂时无法读取图谱</h3><p>检查模型配置与后台服务，然后刷新重试。</p><Button class="graph-button" type="button" onclick={refresh}>重新读取</Button></div>
  {/if}
</section>

<style>
  .knowledge-page{display:flex;flex-direction:column;min-width:0;min-height:0;height:100%;flex:1;position:relative;gap:0;color:var(--app-text)}
  .knowledge-page:focus{outline:none}
  .graph-surface{flex:1;min-width:0;min-height:0;position:relative;overflow:hidden;background:var(--app-surface)}
  .knowledge-page :global(.graph-button){display:inline-flex;align-items:center;justify-content:center;gap:7px;min-height:34px;padding:7px 12px;border:1px solid var(--app-border);border-radius:8px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--font-size-sm);cursor:pointer}.knowledge-page :global(.graph-button:disabled){opacity:.6;cursor:wait}.knowledge-page :global(.graph-button:focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}
  .graph-surface :global(iframe){position:absolute;inset:0;width:100%;height:100%;border:0}.graph-surface.empty{display:none}.notice{margin:12px 16px 0;padding:10px 12px;border:1px solid var(--app-border);border-radius:8px;color:var(--app-muted);font-size:var(--font-size-sm);display:flex;align-items:center;justify-content:space-between;gap:12px}.error{color:var(--color-main-danger-text)}
  .empty-state{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;text-align:center;min-height:320px;padding:24px;border:0;border-radius:0;background:transparent}.empty-state h3{font:var(--font-section);margin:0}.empty-state p{margin:0;max-width:420px;color:var(--app-muted);font-size:var(--font-size-sm);line-height:var(--line-height-body-relaxed)}
</style>
