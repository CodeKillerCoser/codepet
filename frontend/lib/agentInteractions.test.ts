import { describe, expect, it } from "vitest";
import type { Approval, Provider, ProviderCapabilities, TurnTask } from "./generated/runtimeGateway";
import {
  activityActiveTurnFor,
  activityCanResolveApproval,
  activityCapabilitiesFor,
  activityQuickRepliesFor,
} from "./agentInteractions";
import type { PetEvent } from "./types";

function provider(capabilities: Partial<ProviderCapabilities> = {}): Provider {
  return {
    id: "codex",
    providerType: "codex",
    displayName: "Codex Desktop",
    status: "ready",
    capabilities: {
      methods: [],
      permissionLevels: [],
      models: [],
      reasoningEfforts: [],
      quickReplies: [],
      canSteer: false,
      canInterrupt: false,
      ...capabilities,
    },
    extension: {
      namespace: "codepet.codex-desktop",
      data: { source: "codex-desktop-private-ipc" },
    },
  };
}

function turn(status: TurnTask["status"] = "running"): TurnTask {
  return {
    id: "turn-one",
    providerId: "codex",
    conversationId: "thread-one",
    status,
    updatedAt: 1,
  };
}

function approval(status: Approval["status"] = "pending"): Approval {
  return {
    id: "approval-one",
    providerId: "codex",
    conversationId: "thread-one",
    turnId: "turn-one",
    kind: "command-execution",
    title: "Run command",
    status,
    decisions: ["approve", "deny"],
    requestedAt: 1,
  };
}

function event(input: {
  provider: Provider;
  status?: PetEvent["status"];
  turn?: TurnTask | null;
  approval?: Approval | null;
}): PetEvent {
  return {
    id: "event-one",
    provider: "codex",
    kind: "task-updated",
    status: input.status ?? "running",
    title: "Task",
    message: "Running",
    shouldRing: false,
    createdAt: new Date(1).toISOString(),
    raw: null,
    runtimeGateway: {
      provider: input.provider,
      conversationId: "thread-one",
      turn: input.turn,
      approval: input.approval,
    },
  };
}

describe("Runtime Gateway activity capabilities", () => {
  it("exposes steer and interrupt only when the active turn capabilities are advertised", () => {
    const active = event({
      provider: provider({
        methods: ["turn.send", "turn.interrupt"],
        canSteer: true,
        canInterrupt: true,
        quickReplies: [{ id: "continue", label: "继续", text: "请继续。" }],
      }),
      turn: turn(),
    });

    expect(activityActiveTurnFor(active)?.id).toBe("turn-one");
    expect(activityCapabilitiesFor(active)).toMatchObject({
      canReply: true,
      canInterrupt: true,
    });
    expect(activityQuickRepliesFor(active).map((reply) => reply.id)).toEqual(["continue"]);

    const noSteer = event({
      provider: provider({ methods: ["turn.send", "turn.interrupt"], canInterrupt: true }),
      turn: turn(),
    });
    expect(activityCapabilitiesFor(noSteer)).toMatchObject({
      canReply: false,
      canInterrupt: true,
    });
  });

  it("fails closed for a mismatched or terminal turn context", () => {
    const mismatchedTurn = { ...turn(), conversationId: "another-thread" };
    const mismatched = event({
      provider: provider({
        methods: ["turn.send", "turn.interrupt"],
        canSteer: true,
        canInterrupt: true,
      }),
      turn: mismatchedTurn,
    });
    expect(activityCapabilitiesFor(mismatched)).toMatchObject({
      canReply: false,
      canInterrupt: false,
    });

    const terminal = event({
      provider: provider({ methods: ["turn.send", "turn.interrupt"], canInterrupt: true }),
      status: "done",
      turn: turn("completed"),
    });
    expect(activityActiveTurnFor(terminal)).toBeUndefined();
    expect(activityCapabilitiesFor(terminal).canInterrupt).toBe(false);
  });

  it("exposes only pending matching approval decisions", () => {
    const pending = event({
      provider: provider({ methods: ["approval.resolve"] }),
      status: "waiting-approval",
      turn: turn("waiting-approval"),
      approval: approval(),
    });
    expect(activityCanResolveApproval(pending, "approve")).toBe(true);
    expect(activityCanResolveApproval(pending, "deny")).toBe(true);

    const wrongOwner = event({
      provider: provider({ methods: ["approval.resolve"] }),
      status: "waiting-approval",
      approval: { ...approval(), providerId: "another-provider" },
    });
    expect(activityCapabilitiesFor(wrongOwner).canApprove).toBe(false);
    expect(activityCanResolveApproval(wrongOwner, "approve")).toBe(false);
  });

  it("does not expose companion actions for a remote App Server provider", () => {
    const remoteProvider = {
      ...provider({
        methods: ["turn.send", "turn.interrupt", "approval.resolve"],
        canSteer: true,
        canInterrupt: true,
      }),
      extension: {
        namespace: "codex.app-server",
        data: { managedConversationEvents: true },
      },
    };
    const remote = event({
      provider: remoteProvider,
      turn: turn(),
      approval: approval(),
    });

    expect(activityCapabilitiesFor(remote)).toMatchObject({
      canReply: false,
      canInterrupt: false,
      canApprove: false,
    });
  });
});
