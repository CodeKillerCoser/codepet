import { invoke } from "@tauri-apps/api/core";
import { codepetGateway } from "./codepetGateway";

export interface LineageThread { id: string; title: string; workspace: string; createdBy: string; creationKind: string; parentId: string | null; sourceFile: string; timestamp: string | null }
export interface LineageMessage { evidence: { eventId: string; file: string; byteOffset: number; generation: number }; threadId: string; role: string; text: string; timestamp: string | null; turnId: string | null }
export interface Episode { id: string; threadId: string; title: string; evidenceIds: string[]; startedAt: string | null; endedAt: string | null }
export interface LineageTask { id: string; revision: number; rootThreadId: string; title: string; detail: string; manualCompletion: boolean; episodes: Episode[]; edges: { from: string; to: string; evidenceIds: string[] }[] }
export interface LineageSnapshot { threads: LineageThread[]; tasks: LineageTask[]; pendingMessages: number; diagnostics: string[]; lastExtraction?: { requestedModel: string; modelUsage: Record<string, unknown>; reportedCostUsd: number | null } | null }
export interface WorkspaceFacts { workspace: string; repository: string | null; branch: string | null; head: string | null; mainWorkspace: string | null; workMode: string; commitState: string; syncState: string; diagnostic: string | null }
export interface LineageWatch { revision: number; enabled: boolean; extract: boolean; threadId: string | null; model: string; remainingJobs: number; lastError: string | null }
export interface LineageOptions { sources: { id: string; name: string; directory: string | null; watch?: LineageWatch }[]; claudeExecutable: string | null; defaultModel: string; budgetUsd: number }
export const lineageApi = {
  options: () => invoke<LineageOptions>("task_lineage_options"),
  watch: (providerId: string, config: LineageWatch) => invoke<LineageWatch>("task_lineage_watch", { providerId, config }),
  snapshot: (providerId: string) => invoke<LineageSnapshot>("task_lineage_snapshot", { providerId }),
  scan: (providerId: string) => invoke<LineageSnapshot>("task_lineage_scan", { providerId }),
  extract: (providerId: string, threadId: string | null, model: string) => invoke<LineageSnapshot>("task_lineage_extract", { providerId, threadId, model }),
  messages: (providerId: string, threadId: string) => invoke<LineageMessage[]>("task_lineage_messages", { providerId, threadId }),
  workspace: (providerId: string, threadId: string) => invoke<WorkspaceFacts>("task_lineage_workspace", { providerId, threadId }),
  complete: (providerId: string, task: LineageTask, completed: boolean) => invoke<LineageTask>("task_lineage_complete", { providerId, taskId: task.id, revision: task.revision, completed }),
  async status(providerId: string, threadId: string) {
    const result = await codepetGateway("conversation.get", { conversation: { providerId, nativeResourceId: threadId }, limit: 1 });
    return result.conversation.status;
  },
  async send(providerId: string, threadId: string, text: string) {
    const conversation = { providerId, nativeResourceId: threadId };
    const description = await codepetGateway("provider.describe", { providerId });
    if (!description.capabilities.methods.includes("turn.send")) throw new Error("当前实例不支持继续对话");
    const interaction = await codepetGateway("conversation.acquireInteraction", { conversation });
    const result = await codepetGateway("turn.send", { conversation, clientRequestId: crypto.randomUUID(), capabilityRevision: description.capabilities.revision, input: { kind: "text", text }, selection: interaction.selection });
    if (!result.accepted) throw new Error("消息未被接受，请重试");
  },
};

export function rootThread(id: string, threads: LineageThread[]): string {
  const seen = new Set<string>();
  while (!seen.has(id)) {
    seen.add(id);
    const parent = threads.find(t => t.id === id)?.parentId;
    if (!parent || !threads.some(t => t.id === parent)) return id;
    id = parent;
  }
  return [...seen].sort()[0];
}
export function reachable(task: LineageTask, id: string): Set<string> {
  const result = new Set([id]);
  for (const direction of ["up", "down"]) {
    const visited = new Set([id]), pending = [id];
    while (pending.length) {
      const current = pending.pop();
      for (const edge of task.edges) {
        const next = direction === "up" && edge.to === current ? edge.from : direction === "down" && edge.from === current ? edge.to : null;
        if (next && !visited.has(next)) { visited.add(next); result.add(next); pending.push(next); }
      }
    }
  }
  return result;
}
export function taskStatus(task: LineageTask, statuses: Record<string, string>): string {
  if (task.episodes.some(e => ["running", "waiting-approval", "waiting-user-input"].includes(statuses[e.threadId]))) return "进行中";
  if (task.manualCompletion) return "已完成";
  if (!task.episodes.length || task.episodes.some(e => !statuses[e.threadId] || statuses[e.threadId] === "unknown")) return "状态待确认";
  return "待验收";
}
export const creationLabel = (kind: string) => ({ main: "主对话", fork: "Fork", newThread: "新对话", subagent: "Subagent", unknown: "关系待确认" })[kind] ?? "关系待确认";

export function episodePositions(task: LineageTask | undefined, mode: "graph" | "swimlane") {
  if (!task) return {} as Record<string, { x: number; y: number }>;
  const ordered = [...task.episodes].sort((a, b) => (a.startedAt ?? "").localeCompare(b.startedAt ?? ""));
  const lanes = [...new Set(ordered.map(e => e.threadId))], positions: Record<string, { x: number; y: number }> = {};
  const times = ordered.map(e => Date.parse(e.startedAt ?? "")).filter(Number.isFinite);
  const first = Math.min(...times), duration = Math.max(1, Math.max(...times) - first);
  const lastInLane: Record<string, number> = {};
  for (const [index, episode] of ordered.entries()) {
    const parents = task.edges.filter(edge => edge.to === episode.id).map(edge => positions[edge.from]?.x).filter(x => x !== undefined);
    const desired = mode === "graph" ? (parents.length ? Math.max(...parents) + 246 : 20) : 20 + (Number.isFinite(Date.parse(episode.startedAt ?? "")) ? (Date.parse(episode.startedAt!) - first) / duration * Math.max(492, (ordered.length - 1) * 246) : index * 246);
    const x = Math.max(desired, (lastInLane[episode.threadId] ?? -226) + 246);
    positions[episode.id] = { x, y: 30 + lanes.indexOf(episode.threadId) * 155 }; lastInLane[episode.threadId] = x;
  }
  return positions;
}
