import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { listAgentRuntimes } from "./api";
import type { AgentRuntime } from "./types";

export type ProviderConnectionState = {
  providerId: string;
  connectionStatus: "connecting" | "online" | "offline";
  generation: number;
  instances: { id: string; status: string }[];
};

export function observeProviderRuntimes(callbacks: {
  connections: (states: ProviderConnectionState[]) => void;
  runtimes: (runtimes: AgentRuntime[]) => void;
  error: (error: unknown) => void;
}) {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  let unlistenRuntime: (() => void) | undefined;
  let eventVersion = 0;
  let connectionKey: string | undefined;
  let revision = 0;
  let readRuntimes = listAgentRuntimes;
  let pending: Promise<AgentRuntime[]> | undefined;

  async function drain(): Promise<AgentRuntime[]> {
    let snapshot: AgentRuntime[] = [];
    try {
      while (!disposed) {
        const requestedRevision = revision;
        try {
          snapshot = await readRuntimes();
          if (!disposed && requestedRevision === revision) callbacks.runtimes(snapshot);
        } catch (error) {
          if (!disposed && requestedRevision === revision) throw error;
        }
        if (requestedRevision === revision) break;
      }
      return snapshot;
    } finally {
      pending = undefined;
    }
  }

  function refresh(read = listAgentRuntimes): Promise<AgentRuntime[]> {
    if (disposed) return Promise.resolve([]);
    revision++;
    readRuntimes = read;
    // Coalesce state changes during discovery and discard the old result.
    pending ??= drain();
    return pending;
  }

  function acceptConnections(states: ProviderConnectionState[]) {
    if (disposed) return;
    callbacks.connections(states);
    // Harness/turn activity must not repeatedly probe installed executables.
    const key = JSON.stringify(states.map(({ providerId, connectionStatus, generation }) =>
      [providerId, connectionStatus, generation]).sort((a, b) => String(a[0]).localeCompare(String(b[0]))));
    if (key === connectionKey) return;
    connectionKey = key;
    void refresh().catch(callbacks.error);
  }

  async function start(): Promise<void> {
    unlisten = await listen<ProviderConnectionState[]>("provider-connection-status", ({ payload }) => {
      eventVersion++;
      acceptConnections(payload);
    });
    if (disposed) { unlisten(); return; }
    unlistenRuntime = await listen("provider-runtime-changed", () => {
      void refresh().catch(callbacks.error);
    });
    if (disposed) { unlistenRuntime(); return; }
    const version = eventVersion;
    const snapshot = await invoke<ProviderConnectionState[]>("provider_connection_status");
    if (!disposed && version === eventVersion) acceptConnections(snapshot);
    await pending;
  }

  return {
    start,
    refresh,
    dispose() { disposed = true; unlisten?.(); unlistenRuntime?.(); },
  };
}
