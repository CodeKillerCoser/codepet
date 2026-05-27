import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { recentEvents } from "./api";
import type { PetEvent } from "./types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function event(): PetEvent {
  return {
    id: "event-1",
    provider: "codex",
    kind: "task-started",
    status: "thinking",
    title: "任务开始",
    message: "上屏探针",
    sessionId: "session-1",
    cwd: "/workspace",
    toolName: null,
    shouldRing: false,
    createdAt: "2026-05-27T08:18:50.154014Z",
    raw: {},
    source: null,
  };
}

describe("recentEvents", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.mocked(invoke).mockReset();
  });

  it("uses the Tauri command first inside the desktop app", async () => {
    const events = [event()];
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("cross-origin fetch blocked")));
    vi.mocked(invoke).mockResolvedValue(events);

    await expect(recentEvents()).resolves.toEqual(events);

    expect(invoke).toHaveBeenCalledWith("recent_events");
    expect(fetch).not.toHaveBeenCalled();
  });

  it("falls back to the collector endpoint when Tauri IPC is unavailable", async () => {
    const events = [event()];
    vi.mocked(invoke).mockRejectedValue(new Error("IPC unavailable"));
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => events,
    }));

    await expect(recentEvents()).resolves.toEqual(events);

    expect(fetch).toHaveBeenCalledWith("http://127.0.0.1:47621/events");
  });

  it("checks the collector endpoint when Tauri IPC returns an empty startup snapshot", async () => {
    const events = [event()];
    vi.mocked(invoke).mockResolvedValue([]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => events,
    }));

    await expect(recentEvents()).resolves.toEqual(events);

    expect(fetch).toHaveBeenCalledWith("http://127.0.0.1:47621/events");
  });

  it("keeps the empty Tauri snapshot when the collector has no events yet", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    }));

    await expect(recentEvents()).resolves.toEqual([]);
  });

  it("keeps the empty Tauri snapshot when the collector is unavailable", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("collector unavailable")));

    await expect(recentEvents()).resolves.toEqual([]);
  });

  it("falls back to the collector endpoint when Tauri IPC stalls", async () => {
    vi.useFakeTimers();
    const events = [event()];
    vi.mocked(invoke).mockReturnValue(new Promise(() => undefined));
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => events,
    }));

    const result = recentEvents();
    await vi.advanceTimersByTimeAsync(1200);

    await expect(result).resolves.toEqual(events);
    expect(fetch).toHaveBeenCalledWith("http://127.0.0.1:47621/events");
  });
});
