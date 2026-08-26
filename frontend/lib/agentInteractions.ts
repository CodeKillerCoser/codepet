import type { ApprovalDecision, QuickReply } from "./generated/runtimeGateway";
import type { PetEvent } from "./types";

export interface ActivityCapabilities {
  canActivate: boolean;
  canReply: boolean;
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

function runtimeGatewayCapabilities(event: PetEvent): ActivityCapabilities {
  const context = event.runtimeGateway!;
  const provider = context.provider;
  const methods = new Set(provider.capabilities.methods);
  const canSend = provider.status === "ready" && methods.has("turn.send") && Boolean(context.conversationId);
  const isActive = event.status === "thinking" || event.status === "running";
  const canReply =
    canSend &&
    event.status !== "waiting-approval" &&
    (!isActive || (provider.capabilities.canSteer && Boolean(context.turn?.id)));
  const canApprove = Boolean(
    provider.status === "ready" &&
      methods.has("approval.resolve") &&
      context.approval?.status === "pending" &&
      context.approval.decisions.length > 0,
  );
  return {
    canActivate: false,
    canReply,
    canApprove,
    replyReason: canReply ? undefined : provider.status === "ready" ? "当前任务状态不支持继续消息" : "Provider 当前不可用",
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
