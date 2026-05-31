import { describe, expect, it } from "vitest";
import { playWhipSound, shouldRepeatNotification, shouldRing } from "./sound";
import type { AppSettings, PetEvent } from "./types";

function settings(overrides: Partial<AppSettings["notifications"]> = {}): AppSettings {
  return {
    appearance: { theme: "system" },
    pet: {
      selectedPetId: "default",
      kind: "palette",
      sprite: { body: "#111111", accent: "#222222", eyes: "#333333" },
      scale: 3,
      alwaysOnTop: true,
    },
    petLibrary: {
      selectedPetId: "default",
      pets: [],
    },
    notifications: {
      sound: "blip",
      customSoundPath: null,
      ringOnPermission: true,
      ringOnFailure: true,
      ringOnDone: true,
      repeatSeconds: 30,
      quietHoursEnabled: false,
      quietHoursStart: "22:00",
      quietHoursEnd: "08:00",
      ...overrides,
    },
  };
}

function event(overrides: Partial<PetEvent> = {}): PetEvent {
  return {
    id: "event-1",
    provider: "codex",
    kind: "task-completed",
    status: "done",
    title: "任务完成",
    message: "完成",
    sessionId: "session-1",
    cwd: "/tmp/project",
    toolName: null,
    shouldRing: true,
    createdAt: "2026-05-28T00:00:00.000Z",
    raw: {},
    source: null,
    ...overrides,
  };
}

describe("shouldRing", () => {
  it("rings for completed tasks when the done toggle is enabled", () => {
    expect(shouldRing(settings(), event())).toBe(true);
  });

  it("does not ring for completed tasks when the done toggle is disabled", () => {
    expect(shouldRing(settings({ ringOnDone: false }), event())).toBe(false);
  });

  it("does not use done status alone as a completion notification", () => {
    expect(shouldRing(settings(), event({ kind: "message", status: "done", shouldRing: true }))).toBe(false);
  });
});

describe("shouldRepeatNotification", () => {
  it("repeats only while the same permission request is still waiting for approval", () => {
    const permissionEvent = event({
      id: "permission-1",
      kind: "permission-requested",
      status: "waiting-approval",
      title: "等待授权",
      shouldRing: true,
    });
    const refreshedPermissionEvent = event({
      ...permissionEvent,
      id: "permission-2",
      status: "waiting-approval",
      createdAt: "2026-05-28T00:00:05.000Z",
    });

    expect(shouldRepeatNotification(settings(), permissionEvent, [permissionEvent], 1_000, 2_000)).toBe(true);
    expect(shouldRepeatNotification(settings(), permissionEvent, [refreshedPermissionEvent], 1_000, 2_000)).toBe(true);
    expect(shouldRepeatNotification(settings(), permissionEvent, [{ ...permissionEvent, status: "running" }], 1_000, 2_000)).toBe(false);
    expect(shouldRepeatNotification(settings(), permissionEvent, [{ ...permissionEvent, id: "other", sessionId: "other-session" }], 1_000, 2_000)).toBe(false);
    expect(shouldRepeatNotification(settings(), permissionEvent, [], 1_000, 2_000)).toBe(false);
    expect(shouldRepeatNotification(settings(), permissionEvent, [permissionEvent], 2_000, 2_000)).toBe(false);
  });

  it("does not repeat normal task completion sounds", () => {
    const completedEvent = event({
      id: "done-1",
      kind: "task-completed",
      status: "done",
      shouldRing: true,
    });

    expect(shouldRing(settings(), completedEvent)).toBe(true);
    expect(shouldRepeatNotification(settings(), completedEvent, [completedEvent], 1_000, 2_000)).toBe(false);
  });
});

describe("playWhipSound", () => {
  it("is available as a separate pet action sound", () => {
    expect(playWhipSound).toEqual(expect.any(Function));
  });
});
