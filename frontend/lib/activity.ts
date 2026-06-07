import { activityCapabilitiesFor, type ActivityCapabilities } from "./agentInteractions";
import type { ActivityFilterSettings, ActivityKeywordFilterSettings, ActivitySource, PetEvent, TaskStatus } from "./types";

const inactiveStatuses = new Set<TaskStatus>(["idle"]);
const staleActivityStatuses = new Set<TaskStatus>(["thinking", "running"]);
const terminalActivityStatuses = new Set<TaskStatus>(["done", "failed"]);
const activeActivityStaleMs = 30 * 60 * 1000;
const genericActivityTitles = new Set(["任务开始", "收到消息", "正在执行工具", "工具执行完成", "任务完成"]);

export function activityKey(event: PetEvent): string {
  return [event.provider, event.sessionId || event.cwd || "global"].join(":");
}

export function activeActivities(events: PetEvent[], maxItems?: number, now = new Date(), filters?: ActivityFilterSettings): PetEvent[] {
  const activities = new Map<string, PetEvent>();
  const hiddenInternalKeys = new Set<string>();
  const nowMs = now.getTime();

  for (const event of events) {
    applyActivityEvent(activities, event, undefined, hiddenInternalKeys, nowMs, filters);
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
  filters?: ActivityFilterSettings,
): PetEvent[] {
  const activities = new Map(current.map((event) => [activityKey(event), event]));
  const nowMs = now.getTime();
  for (const event of incoming) {
    applyActivityEvent(activities, event, dismissedKeys, hiddenInternalKeys, nowMs, filters);
  }
  return sortActivities(Array.from(activities.values()));
}

export function matchesActivityFilters(event: PetEvent, filters?: ActivityFilterSettings): boolean {
  const agentFilters = activityFiltersForEvent(event, filters);
  const titleKeywords = normalizedKeywords(agentFilters?.titleKeywords);
  const messageKeywords = normalizedKeywords(agentFilters?.messageKeywords);
  if (titleKeywords.length === 0 && messageKeywords.length === 0) {
    return false;
  }

  const titleText = normalizedSearchText(`${event.title}\n${taskTitleFor(event)}`);
  const messageText = normalizedSearchText(event.message);
  return titleKeywords.some((keyword) => titleText.includes(keyword)) || messageKeywords.some((keyword) => messageText.includes(keyword));
}

function activityFiltersForEvent(event: PetEvent, filters?: ActivityFilterSettings): ActivityKeywordFilterSettings | undefined {
  if (!filters) {
    return undefined;
  }

  const byAgent = filters.byAgent ?? {};
  const hasAgentFilters = Object.keys(byAgent).length > 0;
  const agentFilters = byAgent[event.provider];
  if (agentFilters) {
    return agentFilters;
  }
  if (!hasAgentFilters) {
    return {
      titleKeywords: filters.titleKeywords ?? [],
      messageKeywords: filters.messageKeywords ?? [],
    };
  }
  return undefined;
}

function applyActivityEvent(
  activities: Map<string, PetEvent>,
  event: PetEvent,
  dismissedKeys: Set<string> | undefined,
  hiddenInternalKeys: Set<string>,
  nowMs: number,
  filters: ActivityFilterSettings | undefined,
) {
  let key = activityKey(event);
  if (hiddenInternalKeys.has(key)) {
    activities.delete(key);
    return;
  }
  if (isCodexInternalBackgroundEvent(event) || matchesActivityFilters(event, filters)) {
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
    if (hasStableActivityIdentity(event)) {
      return;
    }
    const fallbackKey = latestActiveActivityKeyForProvider(activities, event.provider);
    if (!fallbackKey) {
      return;
    }
    key = fallbackKey;
  }
  const previous = activities.get(key);
  const eventForUpdate = key === activityKey(event) || !previous
    ? event
    : { ...event, sessionId: previous.sessionId, cwd: previous.cwd };
  activities.set(key, displayEventForUpdate(previous, eventForUpdate));
}

function sortActivities(activities: PetEvent[]): PetEvent[] {
  return activities.sort((first, second) => new Date(second.createdAt).getTime() - new Date(first.createdAt).getTime());
}

function displayEventForUpdate(previous: PetEvent | undefined, event: PetEvent): PetEvent {
  const title = authoritativeTitle(event) ?? previousDisplayTitle(previous) ?? taskTitleFor(event);
  const message = previous && isTranscriptPath(event.message) && !isTranscriptPath(previous.message) ? previous.message : event.message;
  const createdAt = shouldRefreshActivitySort(previous, event) ? event.createdAt : previous.createdAt;
  const endedAt = terminalActivityStatuses.has(event.status) ? event.createdAt : null;
  const source = sourceForUpdate(previous?.source, event.source);
  return { ...event, title, message, createdAt, endedAt, source };
}

function previousDisplayTitle(previous: PetEvent | undefined): string | undefined {
  if (!previous) {
    return undefined;
  }
  return authoritativeTitle(previous) ?? taskTitleFor(previous);
}

function shouldRefreshActivitySort(previous: PetEvent | undefined, event: PetEvent): boolean {
  if (!previous) {
    return true;
  }
  return previous.status === "done" && event.status !== "done";
}

function hasStableActivityIdentity(event: PetEvent): boolean {
  return Boolean(event.sessionId || event.cwd);
}

function latestActiveActivityKeyForProvider(activities: Map<string, PetEvent>, provider: PetEvent["provider"]): string | null {
  let latestKey: string | null = null;
  let latestTime = Number.NEGATIVE_INFINITY;
  for (const [key, activity] of activities) {
    if (activity.provider !== provider || terminalActivityStatuses.has(activity.status) || inactiveStatuses.has(activity.status)) {
      continue;
    }
    const createdAt = new Date(activity.createdAt).getTime();
    const createdAtMs = Number.isFinite(createdAt) ? createdAt : 0;
    if (createdAtMs >= latestTime) {
      latestKey = key;
      latestTime = createdAtMs;
    }
  }
  return latestKey;
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

function normalizedKeywords(keywords: string[] | null | undefined): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const keyword of keywords ?? []) {
    const value = normalizedSearchText(keyword);
    if (!value || seen.has(value)) {
      continue;
    }
    seen.add(value);
    normalized.push(value);
  }
  return normalized;
}

function normalizedSearchText(value: string): string {
  return value.trim().toLocaleLowerCase();
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
  return [cardAgentLabel(event), statusLabel(event.status), cardEndTime(event)].filter(Boolean).join(" · ");
}

export function cardSubtitle(event: PetEvent): string {
  return cardMeta(event);
}

export function cardEndTime(event: PetEvent): string {
  if (!terminalActivityStatuses.has(event.status)) {
    return "";
  }
  const timestamp = event.endedAt || event.createdAt;
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) {
    return "";
  }
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

export function activityCapabilities(event: PetEvent): ActivityCapabilities {
  return activityCapabilitiesFor(event);
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

export function cardAgentLabel(event: PetEvent): string {
  return isTerminalSource(event) ? `${event.provider} cli` : event.provider;
}

function isTerminalSource(event: PetEvent): boolean {
  return hasTerminalSourceSignal(event.source);
}

function sourceForUpdate(previous: ActivitySource | null | undefined, incoming: ActivitySource | null | undefined): ActivitySource | null | undefined {
  return sourceRank(incoming) >= sourceRank(previous) ? incoming : previous;
}

function sourceRank(source: ActivitySource | null | undefined): number {
  if (!source) {
    return 0;
  }
  if (hasTerminalSourceSignal(source)) {
    return 2;
  }
  return source.appBundleId ? 1 : 0;
}

function hasTerminalSourceSignal(source: ActivitySource | null | undefined): boolean {
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
  if (isTranscriptPath(event.title)) {
    return statusLabel(event.status);
  }
  if (event.message && event.message !== event.title && !isTranscriptPath(event.message) && genericActivityTitles.has(event.title)) {
    return event.message;
  }
  return event.title;
}

function authoritativeTitle(event: PetEvent): string | null {
  return genericActivityTitles.has(event.title) || isTranscriptPath(event.title) ? null : event.title;
}
