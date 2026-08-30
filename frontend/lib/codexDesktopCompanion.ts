import { invoke } from "@tauri-apps/api/core";
import {
  PROTOCOL_VERSION,
  type ApprovalResolveRequest,
  type ApprovalResolveResponse,
  type Conversation,
  type ConversationCreateRequest,
  type ConversationCreateResponse,
  type ConversationGetRequest,
  type ConversationGetResponse,
  type ConversationListRequest,
  type ConversationListResponse,
  type EventSequence,
  type HandshakeRequest,
  type HandshakeResponse,
  type ProtocolClient,
  type ProtocolError,
  type ProtocolEvent,
  type ProtocolMethod,
  type ProtocolRequest,
  type ProtocolRequestMap,
  type ProtocolResponse,
  type ProtocolResponseMap,
  type Provider,
  type ProviderListRequest,
  type ProviderListResponse,
  type TurnInterruptRequest,
  type TurnInterruptResponse,
  type TurnSendRequest,
  type TurnSendResponse,
} from "./generated/runtimeGateway";

export const codexDesktopCompanionEventName = "codex-desktop-companion-event";

export interface CodexDesktopCompanionSnapshot {
  provider: Provider;
  conversations: Conversation[];
}

let requestSequence = 0;
let clientMessageSequence = 0;

export class CodexDesktopCompanionRequestError extends Error {
  readonly protocolError: ProtocolError;

  constructor(protocolError: ProtocolError) {
    super(protocolError.message);
    this.name = "CodexDesktopCompanionRequestError";
    this.protocolError = protocolError;
  }
}

class TauriCodexDesktopCompanionClient implements ProtocolClient {
  protocolHandshake(request: HandshakeRequest): Promise<HandshakeResponse> {
    return requestCodexDesktopCompanion("protocol.handshake", request);
  }

  providerList(request: ProviderListRequest): Promise<ProviderListResponse> {
    return requestCodexDesktopCompanion("provider.list", request);
  }

  conversationList(request: ConversationListRequest): Promise<ConversationListResponse> {
    return requestCodexDesktopCompanion("conversation.list", request);
  }

  conversationGet(request: ConversationGetRequest): Promise<ConversationGetResponse> {
    return requestCodexDesktopCompanion("conversation.get", request);
  }

  conversationCreate(request: ConversationCreateRequest): Promise<ConversationCreateResponse> {
    return requestCodexDesktopCompanion("conversation.create", request);
  }

  turnSend(request: TurnSendRequest): Promise<TurnSendResponse> {
    return requestCodexDesktopCompanion("turn.send", request);
  }

  turnInterrupt(request: TurnInterruptRequest): Promise<TurnInterruptResponse> {
    return requestCodexDesktopCompanion("turn.interrupt", request);
  }

  approvalResolve(request: ApprovalResolveRequest): Promise<ApprovalResolveResponse> {
    return requestCodexDesktopCompanion("approval.resolve", request);
  }
}

export const codexDesktopCompanionClient: ProtocolClient =
  new TauriCodexDesktopCompanionClient();

export async function readCodexDesktopCompanionSnapshot(): Promise<CodexDesktopCompanionSnapshot> {
  return invoke<CodexDesktopCompanionSnapshot>("codex_desktop_companion_snapshot");
}

export async function replayCodexDesktopCompanionEvents(
  afterEventSequence?: EventSequence,
): Promise<ProtocolEvent[]> {
  return invoke<ProtocolEvent[]>("codex_desktop_companion_replay", {
    afterEventSequence: afterEventSequence ?? null,
  });
}

export function createCodexDesktopCompanionClientMessageId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  clientMessageSequence += 1;
  return `pet-desktop-${Date.now().toString(36)}-${clientMessageSequence.toString(36)}`;
}

export function codexDesktopCompanionErrorMessage(error: unknown): string {
  if (error instanceof CodexDesktopCompanionRequestError) {
    return `[${error.protocolError.code}] ${error.protocolError.message}`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") {
    return error.message;
  }
  return "Codex Desktop companion 请求失败";
}

async function requestCodexDesktopCompanion<M extends ProtocolMethod>(
  method: M,
  params: ProtocolRequestMap[M],
): Promise<ProtocolResponseMap[M]> {
  requestSequence += 1;
  const id = `pet-desktop-${Date.now().toString(36)}-${requestSequence.toString(36)}`;
  const request = {
    protocolVersion: PROTOCOL_VERSION,
    id,
    method,
    params,
  } as ProtocolRequest;
  const response = await invoke<ProtocolResponse>("codex_desktop_companion_request", { request });
  if (response.protocolVersion !== PROTOCOL_VERSION || response.id !== id || response.method !== method) {
    throw new Error(`Codex Desktop companion returned a mismatched response for ${method}`);
  }
  if (response.response.status === "error") {
    throw new CodexDesktopCompanionRequestError(response.response.error);
  }
  return response.response.result as ProtocolResponseMap[M];
}
