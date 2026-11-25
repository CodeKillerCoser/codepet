import { taskRequest } from './taskLedger';
export interface GraphSnapshot { nodes: {id: string; [key: string]: unknown}[]; links: {source: string; target: string; [key: string]: unknown}[]; [key: string]: unknown }
export interface GraphView { detail: {nodes: number; links: number}; destroy(): void }
export const loadRecallGraph = () => taskRequest<GraphSnapshot>('recall-graph');
export async function renderRecallGraph(host: HTMLElement, graph: GraphSnapshot, signal: AbortSignal): Promise<GraphView> {
  const url = new URL(import.meta.env.BASE_URL + 'cognee-view/index.js', document.baseURI).href;
  const {mountCogneeGraph} = await import(/* @vite-ignore */ url);
  return mountCogneeGraph(host, graph, {signal});
}
