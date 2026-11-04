import { describe, expect, it } from "vitest";
import {
  shouldShowRuntime,
  agentRuntimeSourceLabel,
  agentRuntimeStatusMeta,
  canRestoreAutomaticDetection,
  replaceAgentRuntime,
} from "./agentRuntime";
import type { AgentRuntime } from "./types";

function runtime(overrides: Partial<AgentRuntime> = {}): AgentRuntime {
  return {
    providerId: "codex",
    displayName: "Codex",
    status: "ready",
    resolvedExecutable: "/tools/codex",
    source: "current-path",
    configuredExecutable: null,
    version: "codex 1.0",
    diagnostic: null,
    installed: [],
    ...overrides,
  };
}

describe("agent runtime UI state", () => {
  it("maps ready, unavailable, and invalid configuration statuses", () => {
    expect(agentRuntimeStatusMeta(runtime({ status: "loading" }))).toEqual({ label: "启动中", tone: "neutral" });
    expect(agentRuntimeStatusMeta(runtime())).toEqual({ label: "可用", tone: "ready" });
    expect(agentRuntimeStatusMeta(runtime({ status: "unavailable" }))).toEqual({ label: "未检测到", tone: "neutral" });
    expect(agentRuntimeStatusMeta(runtime({ status: "invalid-configured-executable" }))).toEqual({ label: "配置无效", tone: "danger" });
  });

  it("maps detection sources for display", () => {
    expect(agentRuntimeSourceLabel("configured")).toBe("用户配置");
    expect(agentRuntimeSourceLabel("login-shell")).toBe("登录 Shell");
    expect(agentRuntimeSourceLabel("macos-application")).toBe("macOS 应用");
    expect(agentRuntimeSourceLabel(null)).toBe("未解析");
  });

  it("replaces only the runtime returned by a configuration action", () => {
    const claude = runtime({ providerId: "claude", displayName: "Claude Code" });
    const replacement = runtime({ status: "invalid-configured-executable", resolvedExecutable: null });

    expect(replaceAgentRuntime([runtime(), claude], replacement)).toEqual([replacement, claude]);
    expect(canRestoreAutomaticDetection(runtime())).toBe(false);
    expect(canRestoreAutomaticDetection(runtime({ configuredExecutable: "/manual/codex" }))).toBe(true);
  });
});


describe("runtime list visibility", () => {
  const missing = (providerId: string) => runtime({ providerId, status: "unavailable", resolvedExecutable: null, diagnostic: { code: "runtime-not-found", message: "not installed" } });
  it("hides each bundled provider only after confirmed absence", () => {
    for (const id of ["dev.codepet.codex", "dev.codepet.claude", "dev.codepet.opencode"]) {
      expect(shouldShowRuntime(missing(id))).toBe(false);
      expect(shouldShowRuntime({ ...missing(id), status: "loading" })).toBe(true);
      expect(shouldShowRuntime({ ...missing(id), resolvedExecutable: "/tools/codex" })).toBe(true);
    }
  });
  it("preserves custom providers, failures and incompatible installations", () => {
    expect(shouldShowRuntime(missing("custom.provider"))).toBe(true);
    for (const code of ["runtime-scan-failed", "provider-unavailable"]) {
      expect(shouldShowRuntime({ ...missing("dev.codepet.codex"), diagnostic: { code, message: "failure" } })).toBe(true);
    }
    expect(shouldShowRuntime({ ...missing("dev.codepet.codex"), installed: [{ executablePath: "/old/codex", version: "0.1", source: "current-path", incompatibilityReason: "too old" }] })).toBe(true);
  });
});
