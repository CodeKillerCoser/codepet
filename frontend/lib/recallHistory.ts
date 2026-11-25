import { taskRequest } from './taskLedger';
import type { ModelUsage } from './modelGateway';

export type ProcessingState = 'queued' | 'processing' | 'completed' | 'failed' | 'cancelled' | 'interrupted' | 'superseded' | 'paused';
export interface ProcessingUsage {
  models: ModelUsage[]; requests: number; reportedRequests: number;
  inputTokens: number; outputTokens: number; knownTokens: number; complete: boolean;
}
export interface ProcessingResult {
  source?: string; description: string; summaries: string[];
  entities: {id: string; name: string; type: string; description: string}[];
  entityCount?: number; chunkCount?: number; truncated?: boolean;
  indexUpdated?: boolean; snapshotUnavailable?: boolean; note?: string;
}
export interface ProcessingJob {
  id: string; taskId: string; runId: string; title: string; taskTitle: string;
  workspace: string; fingerprint: string; contentHash: string; state: ProcessingState;
  queuedAt: number | null; startedAt: number | null; finishedAt: number | null; elapsedMs: number | null;
  phase: string; error: string | null; resultSummary: string; result?: ProcessingResult | null;
  models: {llm?: string; embedding?: string}; usageTracked: boolean; usage?: ProcessingUsage;
  legacy?: boolean; canCancel: boolean; canRetry: boolean;
}
export interface ProcessingHistory { items: ProcessingJob[]; total: number; counts: Partial<Record<ProcessingState, number>>; offset: number; hasMore: boolean }
export const processingLabels: Record<ProcessingState, string> = {
  queued: '排队中', processing: '加工中', completed: '已完成', failed: '失败',
  cancelled: '已取消', interrupted: '已中断', superseded: '版本已更新', paused: '已暂停',
};
export const historyRequest = <T>(action: string, input: Record<string, unknown> = {}) => taskRequest<T>('recall-history', {action, ...input});
export function duration(milliseconds: number | null) {
  if (milliseconds === null) return '耗时 —';
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
  return `${Math.floor(seconds / 3600)} 小时 ${Math.floor(seconds % 3600 / 60)} 分`;
}
export function completedTime(value: number | null) {
  return value === null ? '完成 —' : new Date(value).toLocaleString('zh-CN', {month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false});
}
export function tokenLabel(job: ProcessingJob) {
  if (job.state === 'queued') return 'Token —';
  if (!job.usageTracked || !job.usage) return 'Token 未报告';
  if (job.state === 'processing' && !job.usage.requests) return 'Token 统计中';
  if (job.state === 'processing') return `已报告 ${job.usage.knownTokens.toLocaleString()} Token`;
  if (!job.usage.complete) return job.usage.reportedRequests ? `${job.usage.knownTokens.toLocaleString()} Token（部分）` : 'Token 未报告';
  return `${job.usage.knownTokens.toLocaleString()} Token`;
}
export function resultLabel(job: ProcessingJob) {
  if (job.state === 'cancelled') return '用户取消排队';
  if (job.state === 'failed') return job.error ? `加工失败 · ${job.error}` : '加工失败';
  if (job.state === 'queued') return '等待加工';
  if (job.state === 'paused') return '模型配置已切换';
  return job.resultSummary || job.phase || (job.state === 'completed' ? '索引已完成，未记录结果快照' : '暂无加工结果');
}
