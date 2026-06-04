import { describe, expect, it } from "vitest";
import { mergeEventFeed } from "./eventFeed";
import type { PetEvent } from "./types";

function event(overrides: Partial<PetEvent>): PetEvent {
  return {
    id: overrides.id ?? "event-a",
    provider: overrides.provider ?? "codex",
    kind: overrides.kind ?? "task-updated",
    status: overrides.status ?? "running",
    title: overrides.title ?? "任务",
    message: overrides.message ?? "消息",
    sessionId: overrides.sessionId ?? "session-a",
    cwd: overrides.cwd ?? "/workspace",
    toolName: overrides.toolName ?? null,
    shouldRing: overrides.shouldRing ?? false,
    createdAt: overrides.createdAt ?? "2026-05-27T06:00:00.000Z",
    raw: overrides.raw ?? {},
    source: overrides.source ?? null,
  };
}

describe("mergeEventFeed", () => {
  it("appends pushed events, deduplicates by id, and keeps newest events last", () => {
    const current = [
      event({ id: "a", createdAt: "2026-05-27T06:00:00.000Z" }),
      event({ id: "b", createdAt: "2026-05-27T06:01:00.000Z", message: "old b" }),
    ];

    const merged = mergeEventFeed(current, [
      event({ id: "b", createdAt: "2026-05-27T06:01:30.000Z", message: "new b" }),
      event({ id: "c", createdAt: "2026-05-27T06:02:00.000Z" }),
    ]);

    expect(merged.map((item) => item.id)).toEqual(["a", "b", "c"]);
    expect(merged[1].message).toBe("new b");
  });
});
