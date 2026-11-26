import { taskRequest } from './taskLedger';
export interface GraphSnapshot { nodes: {id: string; [key: string]: unknown}[]; links: {source: string; target: string; [key: string]: unknown}[]; [key: string]: unknown }
export interface GraphView { detail: {nodes: number; links: number}; destroy(): void }
export const loadRecallGraph = (semantic = false) => taskRequest<GraphSnapshot>('recall-graph', {semantic});
export async function renderRecallGraph(host: HTMLElement, graph: GraphSnapshot, signal: AbortSignal): Promise<GraphView> {
  const url = new URL(import.meta.env.BASE_URL + 'cognee-view/index.js', document.baseURI).href;
  const {mountCogneeGraph} = await import(/* @vite-ignore */ url);
  return mountCogneeGraph(host, graph, {signal, loadSemantic: async () => {
    const semantic = await loadRecallGraph(true);
    if (signal.aborted) throw new Error('图预览已取消');
    if (graph.previewVersion && semantic.previewVersion !== graph.previewVersion) {
      throw new Error('图谱已更新，请刷新后重试');
    }
    return semantic;
  }});
}
