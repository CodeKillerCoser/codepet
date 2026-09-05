import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { listAgentRuntimes } from "./api";
import { observeProviderRuntimes, type ProviderConnectionState } from "./providerRuntimes";
import type { AgentRuntime } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("./api", () => ({ listAgentRuntimes: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function state(connectionStatus: ProviderConnectionState["connectionStatus"], generation = 1): ProviderConnectionState[] {
  return [{ providerId: "dev.codepet.claude", connectionStatus, generation, instances: [] }];
}

function runtime(status: AgentRuntime["status"]): AgentRuntime[] {
  return [{ providerId: "dev.codepet.claude", displayName: "Claude Code", status,
    resolvedExecutable: null, source: null, configuredExecutable: null,
    version: null, installed: [], diagnostic: status === "unavailable"
      ? { code: "provider-unavailable", message: "Provider plugin is not ready" } : null }];
}

describe("Provider lifecycle and runtime inventory", () => {
  let emit: (states: ProviderConnectionState[]) => void;
  let unlisten: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    vi.resetAllMocks();
    unlisten = vi.fn();
    vi.mocked(listen).mockImplementation(async (_event, handler) => {
      emit = (payload) => handler({ payload } as never);
      return unlisten;
    });
    vi.mocked(invoke).mockResolvedValue(state("connecting"));
  });

  function setup() {
    const callbacks = { connections: vi.fn(), runtimes: vi.fn(), error: vi.fn() };
    return { ...callbacks, observer: observeProviderRuntimes(callbacks) };
  }

  it("automatically replaces the startup snapshot after Provider becomes online", async () => {
    vi.mocked(listAgentRuntimes).mockResolvedValueOnce(runtime("loading")).mockResolvedValue(runtime("ready"));
    const { observer, runtimes, error } = setup();
    await observer.start();
    expect(runtimes).toHaveBeenLastCalledWith(runtime("loading"));
    emit(state("online"));
    await vi.waitFor(() => expect(runtimes).toHaveBeenLastCalledWith(runtime("ready")));
    expect(error).not.toHaveBeenCalled();
    observer.dispose();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("ignores a late initial connection snapshot after an online event", async () => {
    const initial = deferred<ProviderConnectionState[]>();
    vi.mocked(invoke).mockReturnValue(initial.promise);
    vi.mocked(listAgentRuntimes).mockResolvedValue(runtime("ready"));
    const { observer, connections, runtimes } = setup();
    const started = observer.start();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
    emit(state("online"));
    initial.resolve(state("connecting"));
    await started;
    expect(connections).toHaveBeenCalledOnce();
    expect(connections).toHaveBeenLastCalledWith(state("online"));
    expect(runtimes).toHaveBeenLastCalledWith(runtime("ready"));
    observer.dispose();
  });

  it("discards a stale inventory response and coalesces lifecycle changes during discovery", async () => {
    const old = deferred<AgentRuntime[]>();
    vi.mocked(listAgentRuntimes).mockReturnValueOnce(old.promise).mockResolvedValue(runtime("ready"));
    const { observer, runtimes } = setup();
    const started = observer.start();
    await vi.waitFor(() => expect(listAgentRuntimes).toHaveBeenCalledOnce());
    emit(state("offline"));
    emit(state("online", 2));
    expect(listAgentRuntimes).toHaveBeenCalledOnce();
    old.resolve(runtime("unavailable"));
    await started;
    expect(listAgentRuntimes).toHaveBeenCalledTimes(2);
    expect(runtimes).toHaveBeenCalledOnce();
    expect(runtimes).toHaveBeenCalledWith(runtime("ready"));
    emit(state("online", 2));
    const harnessReady = state("online", 2);
    harnessReady[0].instances = [{ id: "claude", status: "ready" }];
    emit(harnessReady);
    await Promise.resolve();
    expect(listAgentRuntimes).toHaveBeenCalledTimes(2);
    observer.dispose();
  });

  it("refreshes offline and new generations, preserving real unavailable diagnostics", async () => {
    vi.mocked(invoke).mockResolvedValue(state("online"));
    vi.mocked(listAgentRuntimes).mockResolvedValueOnce(runtime("ready"))
      .mockResolvedValueOnce(runtime("unavailable")).mockResolvedValue(runtime("ready"));
    const { observer, runtimes } = setup();
    await observer.start();
    emit(state("offline"));
    await vi.waitFor(() => expect(runtimes).toHaveBeenLastCalledWith(runtime("unavailable")));
    emit(state("online", 2));
    await vi.waitFor(() => expect(listAgentRuntimes).toHaveBeenCalledTimes(3));
    emit(state("online", 3));
    await vi.waitFor(() => expect(listAgentRuntimes).toHaveBeenCalledTimes(4));
    expect(runtimes).toHaveBeenLastCalledWith(runtime("ready"));
    observer.dispose();
  });

  it("unsubscribes when registration completes after disposal", async () => {
    const registration = deferred<() => void>();
    vi.mocked(listen).mockReturnValue(registration.promise);
    const { observer, connections, runtimes } = setup();
    const started = observer.start();
    observer.dispose();
    registration.resolve(unlisten);
    await started;
    expect(unlisten).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
    expect(connections).not.toHaveBeenCalled();
    expect(runtimes).not.toHaveBeenCalled();
  });
});
