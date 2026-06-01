import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { playWhipSound, shouldPlayWhipReaction, shouldRepeatNotification, shouldRing, whipReactionDelayMs } from "./sound";
import type { AppSettings, PetEvent } from "./types";

function settings(overrides: Partial<AppSettings["notifications"]> = {}): AppSettings {
  return {
    appearance: { theme: "system" },
    pet: {
      selectedPetId: "default",
      kind: "palette",
      sprite: { body: "#111111", accent: "#222222", eyes: "#333333" },
      scale: 3,
      imagePixelSize: 48,
      alwaysOnTop: true,
      whipReactionSound: "none",
      customWhipReactionSoundPath: null,
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

  it("uses the three bundled renamed wav resources for whip cracks", () => {
    const root = resolve(import.meta.dirname, "../..");
    const source = readFileSync(resolve(root, "src/lib/sound.ts"), "utf8");
    const tauriConfig = readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8");
    const resources = [
      "resources/sounds/whip-crack.wav",
      "resources/sounds/whip-swing.wav",
      "resources/sounds/whip-heavy-crack.wav",
    ];

    for (const resource of resources) {
      expect(source).toContain(resource);
      expect(tauriConfig).toContain(resource);
      expect(existsSync(resolve(root, "src-tauri", resource))).toBe(true);
    }
    expect(source).toContain("resolveResource");
  });
});

describe("whip reaction sound", () => {
  it("plays only configured pet reactions after the whip crack", () => {
    expect(shouldPlayWhipReaction("none")).toBe(false);
    expect(shouldPlayWhipReaction(null)).toBe(false);
    expect(shouldPlayWhipReaction("pa")).toBe(true);
    expect(shouldPlayWhipReaction("scream")).toBe(true);
    expect(shouldPlayWhipReaction("custom", null)).toBe(false);
    expect(shouldPlayWhipReaction("custom", "/tmp/ouch.wav")).toBe(true);
  });

  it("delays pet reaction audio until after the whip crack starts", () => {
    expect(whipReactionDelayMs("none")).toBe(0);
    expect(whipReactionDelayMs("pa")).toBeGreaterThan(100);
    expect(whipReactionDelayMs("scream")).toBeGreaterThan(100);
    expect(whipReactionDelayMs("custom", null)).toBe(0);
    expect(whipReactionDelayMs("custom", "/tmp/ouch.wav")).toBeGreaterThan(100);
  });
});
