import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  codexDesktopCompanionClient,
  codexDesktopCompanionEventName,
  codexDesktopCompanionThreadExcludedEventName,
  readCodexDesktopCompanionSnapshot,
  replayCodexDesktopCompanionEvents,
} from "./codexDesktopCompanion";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("Codex Desktop companion transport", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("uses only companion commands and event channels", async () => {
    const snapshot = {
      provider: {
        id: "codex",
        providerType: "codex",
        displayName: "Codex Desktop",
        status: "ready" as const,
        capabilities: {
          methods: ["conversation.get"],
          permissionLevels: [],
          models: [],
          reasoningEfforts: [],
          quickReplies: [],
          canSteer: false,
          canInterrupt: false,
        },
      },
      conversations: [],
    };
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "codex_desktop_companion_snapshot") {
        return snapshot;
      }
      if (command === "codex_desktop_companion_replay") {
        return [];
      }
      const request = (args as { request: {
        protocolVersion: number;
        id: string;
        method: string;
      } }).request;
      return {
        protocolVersion: request.protocolVersion,
        id: request.id,
        method: request.method,
        response: { status: "ok", result: { providers: [snapshot.provider] } },
      };
    });

    await expect(readCodexDesktopCompanionSnapshot()).resolves.toEqual(snapshot);
    await expect(replayCodexDesktopCompanionEvents(7)).resolves.toEqual([]);
    await expect(codexDesktopCompanionClient.providerList({})).resolves.toEqual({
      providers: [snapshot.provider],
    });

    expect(invoke).toHaveBeenCalledWith("codex_desktop_companion_snapshot");
    expect(invoke).toHaveBeenCalledWith("codex_desktop_companion_replay", {
      afterEventSequence: 7,
    });
    expect(invoke).toHaveBeenCalledWith(
      "codex_desktop_companion_request",
      expect.objectContaining({ request: expect.objectContaining({ method: "provider.list" }) }),
    );
    expect(codexDesktopCompanionEventName).toBe("codex-desktop-companion-event");
    expect(codexDesktopCompanionThreadExcludedEventName).toBe(
      "codex-desktop-companion-thread-excluded",
    );
    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).not.toContain(
      "runtime_gateway_request",
    );
  });
});
