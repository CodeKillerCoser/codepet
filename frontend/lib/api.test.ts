import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  appDataDirectory,
  appDataDirectoryTargetStatus,
  checkAppUpdate,
  cutOutImageSubject,
  deletePet,
  getLaunchAtLoginEnabled,
  importPetImage,
  installAppUpdate,
  recentEvents,
  recordPerfEvent,
  setAgentHookEvents,
  setAppDataDirectory,
  setLaunchAtLoginEnabled,
  tokenUsageSummary,
  updatePetImagePixelSize,
} from "./api";
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

  it("keeps the Tauri snapshot when desktop IPC succeeds with no events", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("collector should not be used")));

    await expect(recentEvents()).resolves.toEqual([]);

    expect(fetch).not.toHaveBeenCalled();
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

describe("deletePet", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the desktop command with the selected pet id", async () => {
    const view = { dataDirectory: "/pets", selectedPetId: "default", pets: [] };
    vi.mocked(invoke).mockResolvedValue(view);

    await expect(deletePet("image-custom")).resolves.toEqual(view);

    expect(invoke).toHaveBeenCalledWith("delete_pet", { petId: "image-custom" });
  });
});

describe("appDataDirectory", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("reads the resolved app data directory", async () => {
    vi.mocked(invoke).mockResolvedValue("/tmp/code-pet");

    await expect(appDataDirectory()).resolves.toBe("/tmp/code-pet");

    expect(invoke).toHaveBeenCalledWith("app_data_directory");
  });

  it("checks whether the selected app data directory needs clearing", async () => {
    const status = { isCurrent: false, isEmpty: false, requiresClear: true };
    vi.mocked(invoke).mockResolvedValue(status);

    await expect(appDataDirectoryTargetStatus("/tmp/code-pet")).resolves.toEqual(status);

    expect(invoke).toHaveBeenCalledWith("app_data_directory_target_status", { path: "/tmp/code-pet" });
  });

  it("updates or resets the app data directory", async () => {
    const settings = {
      data: { dataDirectory: "/tmp/code-pet" },
    };
    vi.mocked(invoke).mockResolvedValue(settings);

    await expect(setAppDataDirectory("/tmp/code-pet", true)).resolves.toEqual(settings);
    await setAppDataDirectory(null);

    expect(invoke).toHaveBeenNthCalledWith(1, "set_app_data_directory", {
      path: "/tmp/code-pet",
      clearTarget: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "set_app_data_directory", {
      path: null,
      clearTarget: false,
    });
  });
});

describe("appUpdates", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("checks for an available desktop update", async () => {
    const update = { version: "0.2.0", currentVersion: "0.1.0" };
    vi.mocked(invoke).mockResolvedValue(update);

    await expect(checkAppUpdate()).resolves.toEqual(update);

    expect(invoke).toHaveBeenCalledWith("check_app_update");
  });

  it("installs the pending desktop update", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await expect(installAppUpdate()).resolves.toBeUndefined();

    expect(invoke).toHaveBeenCalledWith("install_app_update");
  });
});

describe("cutOutImageSubject", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the generic desktop subject cutout command", async () => {
    const result = {
      sourcePath: "/tmp/photo.jpg",
      outputPath: "/tmp/photo-subject.png",
      width: 320,
      height: 240,
      mimeType: "image/png",
    };
    vi.mocked(invoke).mockResolvedValue(result);

    await expect(cutOutImageSubject("/tmp/photo.jpg", "/tmp/photo-subject.png")).resolves.toEqual(result);

    expect(invoke).toHaveBeenCalledWith("cut_out_image_subject", {
      sourcePath: "/tmp/photo.jpg",
      outputPath: "/tmp/photo-subject.png",
    });
  });
});

describe("importPetImage", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("passes the requested pixel size to the desktop import command", async () => {
    vi.mocked(invoke).mockResolvedValue({ dataDirectory: "/tmp/pets", selectedPetId: "custom", pets: [] });

    await importPetImage("/tmp/photo.png", "Photo", 72);

    expect(invoke).toHaveBeenCalledWith("import_pet_image", {
      sourcePath: "/tmp/photo.png",
      name: "Photo",
      pixelSize: 72,
    });
  });
});

describe("updatePetImagePixelSize", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the desktop command for the active image pet", async () => {
    vi.mocked(invoke).mockResolvedValue({ dataDirectory: "/tmp/pets", selectedPetId: "custom", pets: [] });

    await updatePetImagePixelSize(64);

    expect(invoke).toHaveBeenCalledWith("update_pet_image_pixel_size", { pixelSize: 64 });
  });
});

describe("tokenUsageSummary", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the desktop token usage summary command", async () => {
    const summary = {
      total: { inputTokens: 10, cachedInputTokens: 0, outputTokens: 2, reasoningOutputTokens: 0, cacheCreationInputTokens: 0, cacheReadInputTokens: 0, totalTokens: 12 },
      byProvider: [],
      byDay: [],
      byBucket: [],
      sessions: [],
    };
    vi.mocked(invoke).mockResolvedValue(summary);

    await expect(tokenUsageSummary()).resolves.toEqual(summary);

    expect(invoke).toHaveBeenCalledWith("token_usage_summary");
  });
});

describe("recordPerfEvent", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the desktop perf logging command", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await recordPerfEvent({
      name: "frontend.main.refresh",
      durationMs: 42.6,
      fields: { agents: 4 },
    });

    expect(invoke).toHaveBeenCalledWith("record_perf_event", {
      event: {
        name: "frontend.main.refresh",
        durationMs: 42.6,
        fields: { agents: 4 },
      },
    });
  });
});

describe("launchAtLogin", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("reads and updates the desktop launch at login command", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(false).mockResolvedValueOnce(true);

    await expect(getLaunchAtLoginEnabled()).resolves.toBe(false);
    await expect(setLaunchAtLoginEnabled(true)).resolves.toBe(true);

    expect(invoke).toHaveBeenNthCalledWith(1, "get_launch_at_login_enabled");
    expect(invoke).toHaveBeenNthCalledWith(2, "set_launch_at_login_enabled", { enabled: true });
  });
});

describe("setAgentHookEvents", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("invokes the desktop command with selected hook events", async () => {
    const agents = [
      {
        id: "codex",
        name: "Codex",
        description: "",
        enabled: true,
        configPath: "/tmp/hooks.json",
        hookEvents: ["UserPromptSubmit", "PreToolUse", "Stop"],
        selectedHookEvents: ["UserPromptSubmit", "Stop"],
      },
    ];
    vi.mocked(invoke).mockResolvedValue(agents);

    await expect(setAgentHookEvents("codex", ["UserPromptSubmit", "Stop"])).resolves.toEqual(agents);

    expect(invoke).toHaveBeenCalledWith("set_agent_hook_events", {
      agentId: "codex",
      hookEvents: ["UserPromptSubmit", "Stop"],
    });
  });
});
