import { invoke } from "@tauri-apps/api/core";
import {
  PROTOCOL_VERSION,
  type ApprovalResolveRequest,
  type ApprovalResolveResponse,
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
  type ProviderListRequest,
  type ProviderListResponse,
  type TurnInterruptRequest,
  type TurnInterruptResponse,
  type TurnSendRequest,
  type TurnSendResponse,
} from "./generated/runtimeGateway";

export const runtimeGatewayEventName = "runtime-gateway-event";

let requestSequence = 0;
let clientMessageSequence = 0;

export class RuntimeGatewayRequestError extends Error {
  readonly protocolError: ProtocolError;

  constructor(protocolError: ProtocolError) {
    super(protocolError.message);
    this.name = "RuntimeGatewayRequestError";
    this.protocolError = protocolError;
  }
}

class TauriRuntimeGatewayClient implements ProtocolClient {
  protocolHandshake(request: HandshakeRequest): Promise<HandshakeResponse> {
    return requestRuntimeGateway("protocol.handshake", request);
  }

  providerList(request: ProviderListRequest): Promise<ProviderListResponse> {
    return requestRuntimeGateway("provider.list", request);
  }

  conversationList(request: ConversationListRequest): Promise<ConversationListResponse> {
    return requestRuntimeGateway("conversation.list", request);
  }

  conversationGet(request: ConversationGetRequest): Promise<ConversationGetResponse> {
    return requestRuntimeGateway("conversation.get", request);
  }

  conversationCreate(request: ConversationCreateRequest): Promise<ConversationCreateResponse> {
    return requestRuntimeGateway("conversation.create", request);
  }

  turnSend(request: TurnSendRequest): Promise<TurnSendResponse> {
    return requestRuntimeGateway("turn.send", request);
  }

  turnInterrupt(request: TurnInterruptRequest): Promise<TurnInterruptResponse> {
    return requestRuntimeGateway("turn.interrupt", request);
  }

  approvalResolve(request: ApprovalResolveRequest): Promise<ApprovalResolveResponse> {
    return requestRuntimeGateway("approval.resolve", request);
  }
}

export const runtimeGatewayClient: ProtocolClient = new TauriRuntimeGatewayClient();

export async function replayRuntimeGatewayEvents(afterEventSequence?: EventSequence): Promise<ProtocolEvent[]> {
  return invoke<ProtocolEvent[]>("runtime_gateway_replay", {
    afterEventSequence: afterEventSequence ?? null,
  });
}

export function createRuntimeGatewayClientMessageId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  clientMessageSequence += 1;
  return `pet-${Date.now().toString(36)}-${clientMessageSequence.toString(36)}`;
}

export function runtimeGatewayErrorMessage(error: unknown): string {
  if (error instanceof RuntimeGatewayRequestError) {
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
  return "Runtime Gateway 请求失败";
}

async function requestRuntimeGateway<M extends ProtocolMethod>(
  method: M,
  params: ProtocolRequestMap[M],
): Promise<ProtocolResponseMap[M]> {
  requestSequence += 1;
  const id = `pet-ui-${Date.now().toString(36)}-${requestSequence.toString(36)}`;
  const request = {
    protocolVersion: PROTOCOL_VERSION,
    id,
    method,
    params,
  } as ProtocolRequest;
  const response = await invoke<ProtocolResponse>("runtime_gateway_request", { request });
  if (response.protocolVersion !== PROTOCOL_VERSION || response.id !== id || response.method !== method) {
    throw new Error(`Runtime Gateway returned a mismatched response for ${method}`);
  }
  if (response.response.status === "error") {
    throw new RuntimeGatewayRequestError(response.response.error);
  }
  return response.response.result as ProtocolResponseMap[M];
}
