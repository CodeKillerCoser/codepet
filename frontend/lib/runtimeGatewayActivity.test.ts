import { describe, expect, it } from "vitest";
import type { Approval, Conversation, ProtocolEvent, Provider, ProviderStatus, TurnTask } from "./generated/runtimeGateway";
import { RuntimeGatewayActivityProjection } from "./runtimeGatewayActivity";

function provider(status: ProviderStatus): Provider {
  return {
    id: "codex",
    providerType: "codex",
    displayName: "Codex Desktop",
    status,
    capabilities: {
      methods: ["conversation.list", "conversation.get"],
      permissionLevels: [],
      models: [],
      reasoningEfforts: [],
      quickReplies: [],
      canSteer: false,
      canInterrupt: false,
    },
  };
}

function turn(status: TurnTask["status"]): TurnTask {
  return {
    id: "turn-one",
    providerId: "codex",
    conversationId: "thread-one",
    status,
    updatedAt: 1_700_000_001_000,
    completedAt: status === "completed" || status === "failed" || status === "interrupted"
      ? 1_700_000_001_000
      : undefined,
  };
}

function conversation(status: Conversation["status"], activeTurn?: TurnTask): Conversation {
  return {
    id: "thread-one",
    providerId: "codex",
    title: "Desktop task",
    status,
    permissionLevel: "workspace-write",
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_001_000,
    activeTurn,
  };
}

function approval(id: string): Approval {
  return {
    id,
    providerId: "codex",
    conversationId: "thread-one",
    turnId: "turn-one",
    kind: "command-execution",
    title: "Run command",
    status: "pending",
    decisions: ["approve", "deny"],
    requestedAt: 1_700_000_001_000,
  };
}

function event(input: Omit<ProtocolEvent, "protocolVersion" | "eventSequence">): ProtocolEvent {
  return {
    protocolVersion: 0,
    eventSequence: 1,
    ...input,
  } as ProtocolEvent;
}

describe("RuntimeGatewayActivityProjection", () => {
  it("clears provider-scoped projection state while the provider is unavailable", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);
    projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("idle") },
    }));
    projection.applyEvent(event({
      event: "turn.upserted",
      payload: { turn: turn("completed") },
    }));

    projection.applyEvent(event({
      event: "provider.statusChanged",
      payload: { provider: provider("unavailable"), previousStatus: "ready" },
    }));
    projection.applyEvent(event({
      event: "provider.statusChanged",
      payload: { provider: provider("ready"), previousStatus: "unavailable" },
    }));
    const result = projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("idle") },
    }));

    expect(result.activities).toHaveLength(1);
    expect(result.activities[0].status).toBe("idle");
    expect(result.activities[0].shouldRing).toBe(false);
  });

  it("does not ring for a conversation baseline error without a live turn transition", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);

    const result = projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("error") },
    }));

    expect(result.activities[0].status).toBe("failed");
    expect(result.activities[0].shouldRing).toBe(false);
  });

  it("projects an interrupted turn as stopped without a failure ring", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);

    const result = projection.applyEvent(event({
      event: "turn.upserted",
      payload: { turn: turn("interrupted") },
    }));

    expect(result.activities[0].status).toBe("idle");
    expect(result.activities[0].message).toBe("任务已停止");
    expect(result.activities[0].shouldRing).toBe(false);
  });

  it("keeps authoritative waiting state when one of multiple approvals resolves", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);
    const waitingTurn = turn("waiting-approval");
    projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("waiting-approval", waitingTurn) },
    }));
    projection.applyEvent(event({
      event: "approval.requested",
      payload: { approval: approval("approval-one") },
    }));
    projection.applyEvent(event({
      event: "approval.requested",
      payload: { approval: approval("approval-two") },
    }));

    const result = projection.applyEvent(event({
      event: "approval.resolved",
      payload: {
        approval: {
          ...approval("approval-one"),
          status: "approved",
          resolvedAt: 1_700_000_002_000,
          decision: "approve",
        },
      },
    }));

    expect(result.activities[0].status).toBe("waiting-approval");
    expect(result.activities[0].runtimeGateway?.approval?.id).toBe("approval-two");
  });

  it("rings once for a pending approval across snapshot resynchronization", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);
    const waitingTurn = turn("waiting-approval");
    const waitingConversation = conversation("waiting-approval", waitingTurn);
    projection.replaceConversations([waitingConversation]);

    const requested = event({
      event: "approval.requested",
      payload: { approval: approval("approval-one") },
    });
    expect(projection.applyEvent(requested).activities[0].shouldRing).toBe(true);

    projection.replaceConversations([waitingConversation]);
    expect(projection.applyEvent(requested).activities[0].shouldRing).toBe(false);

    projection.applyEvent(event({
      event: "approval.resolved",
      payload: {
        approval: {
          ...approval("approval-one"),
          status: "expired",
          resolvedAt: 1_700_000_002_000,
        },
      },
    }));
    expect(projection.applyEvent(requested).activities[0].shouldRing).toBe(true);
  });

  it("permanently excludes an App Server-created thread from companion snapshots and events", () => {
    const projection = new RuntimeGatewayActivityProjection();
    projection.replaceProviders([provider("ready")]);
    projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("running", turn("running")) },
    }));
    projection.applyEvent(event({
      event: "approval.requested",
      payload: { approval: approval("approval-remote") },
    }));

    projection.removeConversation("thread-one");
    const snapshot = projection.replaceConversations([
      conversation("running", turn("running")),
    ]);
    const conversationResult = projection.applyEvent(event({
      event: "conversation.upserted",
      payload: { conversation: conversation("running", turn("running")) },
    }));
    const turnResult = projection.applyEvent(event({
      event: "turn.upserted",
      payload: { turn: turn("running") },
    }));
    const approvalResult = projection.applyEvent(event({
      event: "approval.requested",
      payload: { approval: approval("approval-replayed") },
    }));
    const outputResult = projection.applyEvent(event({
      event: "turn.outputDelta",
      payload: {
        providerId: "codex",
        conversationId: "thread-one",
        turnId: "turn-one",
        outputId: "output-one",
        kind: "agent-message",
        delta: "must stay hidden",
      },
    }));

    expect(snapshot).toEqual([]);
    expect(conversationResult.activities).toEqual([]);
    expect(turnResult.activities).toEqual([]);
    expect(approvalResult.activities).toEqual([]);
    expect(outputResult.activities).toEqual([]);
  });
});
