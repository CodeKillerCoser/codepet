import type { PetEvent, TaskStatus } from "./types";

const inactiveStatuses = new Set<TaskStatus>(["idle"]);
const staleActivityStatuses = new Set<TaskStatus>(["thinking", "running"]);
const activeActivityStaleMs = 30 * 60 * 1000;
const genericActivityTitles = new Set(["任务开始", "收到消息", "正在执行工具", "工具执行完成", "任务完成"]);

export function activityKey(event: PetEvent): string {
  return [event.provider, event.sessionId || event.cwd || "global"].join(":");
}

export function activeActivities(events: PetEvent[], maxItems?: number, now = new Date()): PetEvent[] {
  const activities = new Map<string, PetEvent>();
  const hiddenInternalKeys = new Set<string>();
  const nowMs = now.getTime();

  for (const event of events) {
    applyActivityEvent(activities, event, undefined, hiddenInternalKeys, nowMs);
  }

  const sorted = sortActivities(Array.from(activities.values()));
  if (typeof maxItems === "number") {
    return maxItems <= 0 ? [] : sorted.slice(0, maxItems);
  }
  return sorted;
}

export function updateActivityList(
  current: PetEvent[],
  incoming: PetEvent[],
  dismissedKeys = new Set<string>(),
  now = new Date(),
  hiddenInternalKeys = new Set<string>(),
): PetEvent[] {
  const activities = new Map(current.map((event) => [activityKey(event), event]));
  const nowMs = now.getTime();
  for (const event of incoming) {
    applyActivityEvent(activities, event, dismissedKeys, hiddenInternalKeys, nowMs);
  }
  return sortActivities(Array.from(activities.values()));
}

function applyActivityEvent(
  activities: Map<string, PetEvent>,
  event: PetEvent,
  dismissedKeys: Set<string> | undefined,
  hiddenInternalKeys: Set<string>,
  nowMs: number,
) {
  const key = activityKey(event);
  if (hiddenInternalKeys.has(key)) {
    activities.delete(key);
    return;
  }
  if (isCodexInternalBackgroundEvent(event)) {
    hiddenInternalKeys.add(key);
    activities.delete(key);
    return;
  }
  if (isLifecycleOnlySessionStart(event)) {
    return;
  }
  if (inactiveStatuses.has(event.status) || isStaleActivity(event, nowMs)) {
    activities.delete(key);
    return;
  }
  if (event.status !== "done" && dismissedKeys?.has(key)) {
    dismissedKeys.delete(key);
  }
  if (dismissedKeys?.has(key)) {
    activities.delete(key);
    return;
  }
  if (event.status === "done" && !activities.has(key)) {
    return;
  }
  activities.set(key, displayEventForUpdate(activities.get(key), event));
}

function sortActivities(activities: PetEvent[]): PetEvent[] {
  return activities.sort((first, second) => new Date(second.createdAt).getTime() - new Date(first.createdAt).getTime());
}

function displayEventForUpdate(previous: PetEvent | undefined, event: PetEvent): PetEvent {
  const title = authoritativeTitle(event) ?? previous?.title ?? taskTitleFor(event);
  const message = previous && isTranscriptPath(event.message) && !isTranscriptPath(previous.message) ? previous.message : event.message;
  const createdAt = shouldRefreshActivitySort(previous, event) ? event.createdAt : previous.createdAt;
  return { ...event, title, message, createdAt };
}

function shouldRefreshActivitySort(previous: PetEvent | undefined, event: PetEvent): boolean {
  if (!previous) {
    return true;
  }
  return previous.status === "done" && event.status !== "done";
}

function isTranscriptPath(value: string): boolean {
  return /\/\.(codex|claude)\/.+\.jsonl$/.test(value);
}

function isLifecycleOnlySessionStart(event: PetEvent): boolean {
  if (event.provider !== "codex" || event.kind !== "task-started" || event.status !== "thinking") {
    return false;
  }
  const title = event.title.trim();
  const message = event.message.trim();
  return title === "SessionStart" || message === "SessionStart" || isTranscriptPath(message);
}

function isCodexInternalBackgroundEvent(event: PetEvent): boolean {
  if (event.provider !== "codex") {
    return false;
  }
  const text = `${event.title}\n${event.message}`;
  return (
    text.includes("Generate 0 to 3 hyperpersonalized suggestions for what this user can do with Codex") ||
    text.includes("Recent Codex threads in this project:") ||
    text.includes("Avoid repeating these previously dismissed suggestions:") ||
    text.includes("Each suggestion must include: title, description, prompt, appId") ||
    text.includes("You will be presented with a user prompt, and your job is to provide a short title for a task")
  );
}

function isStaleActivity(event: PetEvent, nowMs: number): boolean {
  if (!staleActivityStatuses.has(event.status)) {
    return false;
  }
  const createdAtMs = new Date(event.createdAt).getTime();
  return Number.isFinite(createdAtMs) && nowMs - createdAtMs > activeActivityStaleMs;
}

export function statusLabel(status: TaskStatus): string {
  switch (status) {
    case "thinking":
      return "正在思考";
    case "running":
      return "正在执行";
    case "waiting-approval":
      return "等待授权";
    case "failed":
      return "需要查看";
    case "done":
      return "任务完成";
    case "idle":
    default:
      return "空闲";
  }
}

export function cardTitle(event: PetEvent): string {
  return taskTitleFor(event);
}

export function cardMessage(event: PetEvent): string {
  if (event.message && event.message !== cardTitle(event)) {
    return event.message;
  }
  if (event.toolName) {
    return `工具：${event.toolName}`;
  }
  if (event.message) {
    return event.message;
  }
  return statusLabel(event.status);
}

export function cardMeta(event: PetEvent): string {
  return `${activitySourceLabel(event)} · ${statusLabel(event.status)}`;
}

export function cardSubtitle(event: PetEvent): string {
  return cardMeta(event);
}

export interface ActivityCapabilities {
  canActivate: boolean;
  canReply: boolean;
  canApprove: boolean;
  replyReason?: string;
}

export function activityCapabilities(event: PetEvent): ActivityCapabilities {
  const terminalProgram = event.source?.terminalProgram ?? "";
  const hasTargetableTerminal = Boolean(event.source?.ttyPath && isSupportedReplyTerminal(terminalProgram));
  const canReply = event.provider === "qoder" && hasTargetableTerminal;
  return {
    canActivate: true,
    canReply,
    canApprove: event.status === "waiting-approval" && event.provider !== "codex",
    replyReason: canReply ? undefined : "来源不支持可靠回复",
  };
}

export function primaryActivity(activities: PetEvent[]): PetEvent | null {
  return [...activities].sort((first, second) => activityPriority(second) - activityPriority(first)).at(0) ?? null;
}

function activityPriority(event: PetEvent): number {
  switch (event.status) {
    case "waiting-approval":
      return 50;
    case "failed":
      return 40;
    case "running":
      return 30;
    case "thinking":
      return 20;
    default:
      return 0;
  }
}

function isSupportedReplyTerminal(program: string): boolean {
  return ["Apple_Terminal", "Terminal", "Terminal.app", "iTerm.app", "iTerm2", "iTerm2.app"].includes(program);
}

function activitySourceLabel(event: PetEvent): string {
  return isTerminalSource(event) ? `${event.provider} cli` : event.provider;
}

function isTerminalSource(event: PetEvent): boolean {
  const source = event.source;
  if (!source) {
    return false;
  }
  return Boolean(
    source.terminalProgram ||
      source.ttyPath ||
      source.tmuxPane ||
      source.weztermPane ||
      source.kittyWindowId,
  );
}

function taskTitleFor(event: PetEvent): string {
  if (event.message && event.message !== event.title && !isTranscriptPath(event.message) && genericActivityTitles.has(event.title)) {
    return event.message;
  }
  return event.title;
}

function authoritativeTitle(event: PetEvent): string | null {
  return genericActivityTitles.has(event.title) ? null : event.title;
}
