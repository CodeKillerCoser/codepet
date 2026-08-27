import type { AgentRuntime, AgentRuntimeSource, AgentRuntimeStatus } from "./types";

export type AgentRuntimeTone = "ready" | "neutral" | "danger";

const statusMetadata: Record<AgentRuntimeStatus, { label: string; tone: AgentRuntimeTone }> = {
  ready: { label: "可用", tone: "ready" },
  unavailable: { label: "未检测到", tone: "neutral" },
  "invalid-configured-executable": { label: "配置无效", tone: "danger" },
};

const sourceLabels: Record<AgentRuntimeSource, string> = {
  configured: "用户配置",
  environment: "环境变量",
  "current-path": "当前 PATH",
  "login-shell": "登录 Shell",
  "macos-application": "macOS 应用",
  "windows-application": "Windows 安装",
};

export function agentRuntimeStatusMeta(runtime: AgentRuntime): { label: string; tone: AgentRuntimeTone } {
  return statusMetadata[runtime.status];
}

export function agentRuntimeSourceLabel(source: AgentRuntimeSource | null | undefined): string {
  return source ? sourceLabels[source] : "未解析";
}

export function replaceAgentRuntime(runtimes: AgentRuntime[], replacement: AgentRuntime): AgentRuntime[] {
  const index = runtimes.findIndex((runtime) => runtime.providerId === replacement.providerId);
  if (index < 0) {
    return [...runtimes, replacement];
  }
  return runtimes.map((runtime, runtimeIndex) => runtimeIndex === index ? replacement : runtime);
}

export function canRestoreAutomaticDetection(runtime: AgentRuntime): boolean {
  return Boolean(runtime.configuredExecutable);
}
