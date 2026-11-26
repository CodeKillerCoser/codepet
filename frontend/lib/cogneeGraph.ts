import { taskRequest } from './taskLedger';
interface PreviewMetadata { previewVersion?: string; previewStatus?: {updating: boolean; pending: number; processing: number; ready: number} }
export interface GraphSnapshot extends PreviewMetadata { nodes: {id: string; [key: string]: unknown}[]; links: {source: string; target: string; [key: string]: unknown}[]; unchanged?: false; [key: string]: unknown }
export interface GraphUnchanged extends PreviewMetadata { unchanged: true; semanticPending?: boolean }
export interface GraphView { detail: {nodes: number; links: number}; destroy(): void; getState?(): Record<string, unknown> }
export const loadRecallGraph = (semantic = false, knownVersion?: string) => taskRequest<GraphSnapshot | GraphUnchanged>('recall-graph', {semantic, ...(knownVersion ? {knownVersion} : {})});
export async function renderRecallGraph(host: HTMLElement, graph: GraphSnapshot, signal: AbortSignal, viewState?: Record<string, unknown>): Promise<GraphView> {
  const url = new URL(import.meta.env.BASE_URL + 'cognee-view/index.js', document.baseURI).href;
  const {mountCogneeGraph} = await import(/* @vite-ignore */ url);
  return mountCogneeGraph(host, graph, {signal, viewState, loadSemantic: async () => {
    const semantic = await loadRecallGraph(true, graph.previewVersion);
    if (signal.aborted) throw new Error('图预览已取消');
    if (semantic.semanticPending) throw new Error('语义布局正在后台准备');
    if (semantic.unchanged) throw new Error('语义快照无效');
    if (graph.previewVersion && semantic.previewVersion !== graph.previewVersion) {
      throw new Error('图谱已更新，请刷新后重试');
    }
    return semantic;
  }});
}
