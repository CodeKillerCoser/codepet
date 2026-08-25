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
  return interactionForEvent(event).capabilities(event);
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
