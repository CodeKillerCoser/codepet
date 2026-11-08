import { invoke } from "@tauri-apps/api/core";
import type { ProtocolMethod, ProtocolRequestMap, ProtocolResponseMap, ProtocolResponse } from "../../sdk/typescript/codepet-gateway-sdk/src/generated";

export class GatewayRequestError extends Error {
  constructor(message: string, public code: string, public retryable = false) { super(message); }
}
let sequence = 0;
/** Canonical Gateway JSON-RPC, shared with remote clients. */
export async function codepetGateway<M extends ProtocolMethod>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]> {
  const id = `codepet-ui-${Date.now()}-${++sequence}`;
  const response = await invoke<ProtocolResponse>("codepet_gateway_request", { request: { jsonrpc: "2.0", id, method, params } });
  if (response.jsonrpc !== "2.0" || response.id !== id) throw new Error("Gateway 返回了不匹配的响应");
  if ("error" in response) throw new GatewayRequestError(response.error.message, String(response.error.data?.code ?? response.error.code), Boolean(response.error.data?.retryable));
  return response.result as ProtocolResponseMap[M];
}
