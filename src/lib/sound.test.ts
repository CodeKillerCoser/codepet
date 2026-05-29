import { describe, expect, it } from "vitest";
import { shouldRing } from "./sound";
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
