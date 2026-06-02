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

const codexRemoteInteraction: AgentInteraction = {
  capabilities(event) {
    const isActiveConversation = event.status === "thinking" || event.status === "running";
    const canReply = isActiveConversation && Boolean(event.sessionId);
    return {
      canActivate: true,
      canReply,
      canApprove: event.status === "waiting-approval",
      replyReason: canReply ? undefined : "来源不支持可靠回复",
    };
  },
};

const qoderInteraction: AgentInteraction = {
  capabilities(event) {
    const terminalProgram = event.source?.terminalProgram ?? "";
    const hasTargetableTerminal = Boolean(event.source?.ttyPath && isSupportedReplyTerminal(terminalProgram));
    const isActiveConversation = event.status === "thinking" || event.status === "running";
    const canReply = isActiveConversation && hasTargetableTerminal;
    return {
      canActivate: true,
      canReply,
      canApprove: event.status === "waiting-approval",
      replyReason: canReply ? undefined : "来源不支持可靠回复",
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
      return codexRemoteInteraction;
    case "qoder":
      return qoderInteraction;
    default:
      return defaultInteraction;
  }
}

function isSupportedReplyTerminal(program: string): boolean {
  return ["Apple_Terminal", "Terminal", "Terminal.app", "iTerm.app", "iTerm2", "iTerm2.app"].includes(program);
}
