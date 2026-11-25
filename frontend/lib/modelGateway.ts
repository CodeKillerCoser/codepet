import { invoke } from "@tauri-apps/api/core";
export interface ModelProfile { endpoint: string; model: string; apiKey: string; hasApiKey?: boolean; dimensions: number }
export interface ModelSettings { enabled: boolean; llm: ModelProfile; embedding: ModelProfile; plainJson: boolean; llmExtraBody: Record<string, unknown> }
export interface ModelUsage { purpose: string; model: string; requests: number; succeeded: number; inputTokens: number; outputTokens: number; reportedRequests: number }
export interface ModelConfiguration { settings: ModelSettings; usage: { models: ModelUsage[] } }
export function modelGatewayRequest(op: string, settings?: ModelSettings): Promise<ModelConfiguration> {
  return invoke("task_ledger_request", { request: { op, settings } });
}
