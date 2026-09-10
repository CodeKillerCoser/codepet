// Browser-only QA fixture. This entry is excluded from production build inputs.
import { mockIPC } from "@tauri-apps/api/mocks";
import { mount } from "svelte";
import TaskWorkspaceQa from "./TaskWorkspaceQa.svelte";
import "../styles.css";
import "../main-window.css";
import type { LineageSnapshot, LineageMessage } from "../lib/taskLineage";
let watch = { revision: 0, enabled: false, extract: false, threadId: null as string | null, model: "haiku", remainingJobs: 5, lastError: null };
let config = { revision: 0, harness: "claude", harnessInstanceId: null, model: "haiku", skill: "extract-tasks", prompt: "", budgetUsd: 0.25, timeoutSeconds: 90, intervalSeconds: 60, debounceSeconds: 20 };
let jobs: any[] = [];
const timestamp = "2026-09-10T04:00:00Z";
const data: LineageSnapshot = {
  threads: [
    { id: "main", title: "构建 Codex 任务谱系管理", workspace: "D:/Projects/CodePet", createdBy: "human", creationKind: "main", parentId: null, sourceFile: "/fixture/main.jsonl", timestamp },
    { id: "child", title: "验证增量读取与证据边界", workspace: "D:/Worktrees/CodePet-reader", createdBy: "agent", creationKind: "newThread", parentId: "main", sourceFile: "/fixture/child.jsonl", timestamp },
  ], tasks: [{ id: "task", revision: 1, rootThreadId: "main", title: "任务谱系第一版", detail: "从原始对话识别可验收任务，追踪跨线程执行过程，并回到消息继续处理。", manualCompletion: false,
    episodes: [
      { id: "start", threadId: "main", title: "确定任务抽取边界", evidenceIds: ["m1", "m2"], startedAt: timestamp, endedAt: timestamp },
      { id: "read", threadId: "child", title: "验证增量读取", evidenceIds: ["c1", "c2"], startedAt: "2026-09-10T04:10:00Z", endedAt: "2026-09-10T04:30:00Z" },
      { id: "finish", threadId: "main", title: "整合并请求验收", evidenceIds: ["m3"], startedAt: "2026-09-10T04:40:00Z", endedAt: "2026-09-10T04:50:00Z" },
    ], edges: [{ from: "start", to: "read", evidenceIds: ["m2"] }, { from: "read", to: "finish", evidenceIds: ["m3"] }] }], pendingMessages: 0, diagnostics: [],
  lastExtraction: { requestedModel: "haiku", modelUsage: { "fixture-low-cost-model": {} }, reportedCostUsd: null },
};
const history: LineageMessage[] = [
  ["m1", "main", "user", "任务抽取只把用户消息和 AI 正文作为对象，工具执行不要。"],
  ["m2", "main", "assistant", "已将这一边界写入来源解析和执行器校验。下一步验证增量追加与恢复。"],
  ["c1", "child", "user", "验证半行追加、同长度改写和工具文本排除。"],
  ["c2", "child", "assistant", "三项来源回归测试已通过，证据位置保持稳定。"],
  ["m3", "main", "assistant", "代码已经整合，等待人工确认第一版验收。"],
].map(([id, threadId, role, text], index) => ({ evidence: { eventId: id, file: `/fixture/${threadId}.jsonl`, byteOffset: index * 200, generation: 0 }, threadId, role, text, timestamp, turnId: id }));
mockIPC((command, raw) => {
  const args = (raw ?? {}) as Record<string, any>;
  if (command === "task_lineage_options") return { sources: [{ id: "codex-fixture", name: "Codex · QA 模拟数据", directory: "/fixture", watch, extraction: config }], claudeExecutable: "/fixture/claude", defaultModel: "haiku", budgetUsd: 0.25, instances: [{ id: "claude", name: "本机 Claude", harness: "claude" }], layout: {root: "/fixture/.codepair", skills: "/fixture/.codepair/skills", extractionWorkspaces: "/fixture/.codepair/workspaces/task-extraction", taskWorkspaces: "/fixture/.codepair/workspaces/tasks"} };
  if (command === "task_lineage_request") {
    const r = args.request;
    if (r.method === "tasks.settings.get") return structuredClone(config);
    if (r.method === "tasks.settings.set") { config = { ...r.config, revision: config.revision + 1 }; return structuredClone(config); }
    if (r.method === "tasks.skills") return ["extract-tasks", "reconcile-tasks"];
    if (r.method === "tasks.skill.get") return { name: r.name, text: `---\nname: ${r.name}\ndescription: Task extraction\n---\n只抽取最近 48 小时用户消息与 AI 正文。` };
    if (r.method === "tasks.dirty") return [{threadId: "main", state: "clean", revision: 1, extractedRevision: 1, pendingMessages: 0, lastChangedAt: Date.now(), lastError: null}];
    if (r.method === "tasks.jobs") return structuredClone(jobs);
    if (r.method === "tasks.extract") { const job = {id: r.requestId, requestId: r.requestId, threadId: r.threadId, state: "queued", createdAt: Date.now(), finishedAt: null, error: null}; jobs = [job]; setTimeout(() => { job.state = "completed"; }, 500); return structuredClone(job); }
    if (r.method === "tasks.workspace.request") return {taskId: r.taskId, path: "/fixture/.codepair/workspaces/tasks/task"};
    throw new Error(`Unhandled task method: ${r.method}`);
  }
  if (command === "task_lineage_watch") { watch = { ...args.config, revision: watch.revision + 1 }; return watch; }
  if (["task_lineage_snapshot", "task_lineage_scan", "task_lineage_extract"].includes(command)) return structuredClone(data);
  if (command === "task_lineage_messages") return history.filter(m => m.threadId === args.threadId);
  if (command === "task_lineage_workspace") return { workspace: data.threads.find(t => t.id === args.threadId)?.workspace, repository: "D:/Projects/CodePet", branch: "codex/task-lineage-v1", head: "a81b0c9d4e52", mainWorkspace: "D:/Projects/CodePet", workMode: args.threadId === "child" ? "worktree" : "main", commitState: "clean", syncState: args.threadId === "child" ? "synced" : "notRequired", diagnostic: null };
  if (command === "task_lineage_complete") { data.tasks[0].manualCompletion = args.completed; data.tasks[0].revision++; return structuredClone(data.tasks[0]); }
  if (command === "codepet_gateway_request") {
    const request = args.request;
    let result: any;
    if (request.method === "conversation.get") result = { conversation: { status: "idle" }, items: [] };
    else if (request.method === "provider.describe") result = { capabilities: { revision: "fixture", methods: ["turn.send"] } };
    else if (request.method === "conversation.acquireInteraction") result = { selection: {} };
    else if (request.method === "turn.send") result = { accepted: true };
    else throw new Error(`Unhandled fixture gateway method: ${request.method}`);
    return { jsonrpc: "2.0", id: request.id, result };
  }
  throw new Error(`Unhandled fixture command: ${command}`);
}, { shouldMockEvents: true });
mount(TaskWorkspaceQa, { target: document.getElementById("app")! });
