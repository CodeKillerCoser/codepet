import { invoke } from "@tauri-apps/api/core";
import type { DeviceDescriptor } from "../../sdk/typescript/codepet-core-sdk/src/generated";

export type RemoteAccessRuntimePhase = "starting" | "available" | "unavailable" | "stopping" | "stopped";

export interface RemoteAccessDiagnostic {
  code: string;
  message: string;
  retryable: boolean;
}

export interface RemoteAccessStatus {
  phase: RemoteAccessRuntimePhase;
  hostDeviceId?: string | null;
  displayName?: string | null;
  advertisedHost?: string | null;
  networkInterface?: { index: number; name: string; kind: string; ipv4: string; physical: boolean; available: boolean } | null;
  httpsBaseUrl?: string | null;
  activeSessionCount: number;
  pairingAvailable: boolean;
  diagnostic?: RemoteAccessDiagnostic | null;
}

export interface RemoteClient {
  credentialId: string;
  remoteClientId: string;
  descriptor: DeviceDescriptor;
  createdAt: number;
  lastSeenAt: number;
  revokedAt?: number | null;
  onlineSessionCount: number;
}

export interface RemotePairingStart {
  pairingId: string;
  expiresAt: number;
  qrSvgDataUrl: string;
}

export type RemotePairingStatusKind = "active" | "succeeded" | "expired" | "cancelled";

export interface RemotePairingStatus {
  pairingId: string;
  state: RemotePairingStatusKind;
  expiresAt: number;
  qrSvgDataUrl?: string | null;
}

export interface IncomingRemotePairingRequest {
  requestId: string;
  remoteClientId: string;
  descriptor: DeviceDescriptor;
  expiresAt: number;
  confirmationCode: string;
}

export interface RemoteCredentialRevokeResult {
  credentialId: string;
  revokedAt?: number | null;
  disconnectedSessionCount: number;
}

export async function getRemoteAccessStatus(): Promise<RemoteAccessStatus> {
  return invoke<RemoteAccessStatus>("remote_access_status");
}

export async function retryRemoteAccess(): Promise<RemoteAccessStatus> {
  return invoke<RemoteAccessStatus>("retry_remote_access");
}

export async function listRemoteClients(): Promise<RemoteClient[]> {
  return invoke<RemoteClient[]>("list_remote_clients");
}

export async function startRemotePairing(): Promise<RemotePairingStart> {
  return invoke<RemotePairingStart>("start_remote_pairing");
}

export async function getRemotePairingStatus(pairingId: string): Promise<RemotePairingStatus> {
  return invoke<RemotePairingStatus>("get_remote_pairing_status", { pairingId });
}

export async function copyRemotePairingJson(pairingId: string): Promise<void> {
  await invoke("copy_remote_pairing_json", { pairingId });
}

export async function cancelRemotePairing(pairingId: string): Promise<RemotePairingStatus> {
  return invoke<RemotePairingStatus>("cancel_remote_pairing", { pairingId });
}

export async function listRemotePairingRequests(): Promise<IncomingRemotePairingRequest[]> {
  return invoke<IncomingRemotePairingRequest[]>("list_remote_pairing_requests");
}

export async function resolveRemotePairingRequest(requestId: string, accept: boolean): Promise<void> {
  await invoke("resolve_remote_pairing_request", { requestId, accept });
}

export async function revokeRemoteCredential(credentialId: string): Promise<RemoteCredentialRevokeResult> {
  return invoke<RemoteCredentialRevokeResult>("revoke_remote_credential", { credentialId });
}

export function remoteCommandDiagnostic(error: unknown, fallbackCode: string): RemoteAccessDiagnostic {
  if (typeof error === "object" && error !== null) {
    const candidate = error as Record<string, unknown>;
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      return {
        code: candidate.code,
        message: candidate.message,
        retryable: candidate.retryable === true,
      };
    }
  }

  return {
    code: fallbackCode,
    message: "Remote Host 暂时无法完成此操作。",
    retryable: true,
  };
}
