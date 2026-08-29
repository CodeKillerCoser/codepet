import type { ApprovalDecision, QuickReply, TurnTask } from "./generated/runtimeGateway";
import type { PetEvent } from "./types";

export interface ActivityCapabilities {
  canActivate: boolean;
  canReply: boolean;
  canInterrupt: boolean;
  canApprove: boolean;
  replyReason?: string;
}

interface AgentInteraction {
  capabilities(event: PetEvent): ActivityCapabilities;
}

const codexLegacyInteractionDisabled: AgentInteraction = {
  capabilities() {
    return {
      canActivate: false,
      canReply: false,
      canInterrupt: false,
      canApprove: false,
      replyReason: "Codex 旧活动源已停用",
    };
  },
};

const qoderInteraction: AgentInteraction = {
  capabilities(event) {
    const canReply = false;
    const canApprove = event.status === "waiting-approval";
    return {
      canActivate: true,
      canReply,
      canInterrupt: false,
      canApprove,
      replyReason: "Qoder cannot send messages to existing local sessions yet",
    };
  },
};

const defaultInteraction: AgentInteraction = {
  capabilities(event) {
    return {
      canActivate: true,
      canReply: false,
      canInterrupt: false,
      canApprove: event.status === "waiting-approval",
      replyReason: "来源不支持可靠回复",
    };
  },
};

export function activityCapabilitiesFor(event: PetEvent): ActivityCapabilities {
  if (event.runtimeGateway) {
    return runtimeGatewayCapabilities(event);
  }
  return interactionForEvent(event).capabilities(event);
}

export function activityQuickRepliesFor(event: PetEvent): QuickReply[] {
  return activityCapabilitiesFor(event).canReply
    ? event.runtimeGateway?.provider.capabilities.quickReplies ?? []
    : [];
}

export function activityCanResolveApproval(event: PetEvent, decision: ApprovalDecision): boolean {
  const approval = event.runtimeGateway?.approval;
  return Boolean(
    activityCapabilitiesFor(event).canApprove &&
      approval?.status === "pending" &&
      approval.decisions.includes(decision),
  );
}

export function activityActiveTurnFor(event: PetEvent): TurnTask | undefined {
  const context = event.runtimeGateway;
  const turn = context?.turn;
  if (
    !context ||
    !turn ||
    turn.providerId !== context.provider.id ||
    turn.conversationId !== context.conversationId ||
    turn.status === "completed" ||
    turn.status === "failed" ||
    turn.status === "interrupted"
  ) {
    return undefined;
  }
  return turn;
}

function runtimeGatewayCapabilities(event: PetEvent): ActivityCapabilities {
  const context = event.runtimeGateway!;
  const provider = context.provider;
  const methods = new Set(provider.capabilities.methods);
  const turnMatchesContext = !context.turn || (
    context.turn.providerId === provider.id && context.turn.conversationId === context.conversationId
  );
  const activeTurn = activityActiveTurnFor(event);
  const canSend = Boolean(
    provider.status === "ready" &&
      methods.has("turn.send") &&
      context.conversationId &&
      turnMatchesContext,
  );
  const canReply =
    canSend &&
    event.status !== "waiting-approval" &&
    (!activeTurn || provider.capabilities.canSteer);
  const canInterrupt = Boolean(
    provider.status === "ready" &&
      methods.has("turn.interrupt") &&
      provider.capabilities.canInterrupt &&
      activeTurn,
  );
  const approvalMatchesContext = Boolean(
    context.approval?.providerId === provider.id &&
      context.approval.conversationId === context.conversationId,
  );
  const canApprove = Boolean(
    provider.status === "ready" &&
      methods.has("approval.resolve") &&
      approvalMatchesContext &&
      context.approval?.status === "pending" &&
      context.approval.decisions.length > 0,
  );
  return {
    canActivate: false,
    canReply,
    canInterrupt,
    canApprove,
    replyReason: canReply
      ? undefined
      : provider.status !== "ready"
        ? "Provider 当前不可用"
        : !methods.has("turn.send")
          ? "Provider 未声明 turn.send capability"
          : "当前任务状态不支持继续消息",
  };
}

function interactionForEvent(event: PetEvent): AgentInteraction {
  switch (event.provider) {
    case "codex":
      return codexLegacyInteractionDisabled;
    case "qoder":
      return qoderInteraction;
    default:
      return defaultInteraction;
  }
}
