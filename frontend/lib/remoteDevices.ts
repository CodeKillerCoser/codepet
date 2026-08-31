import type { RemoteClient, RemotePairingStatusKind } from "./remoteAccess";

export type RemoteDeviceType = "phone" | "tablet" | "desktop" | "browser" | "unknown";

export type RemoteDeviceStatus = "online" | "offline" | "never-connected" | "revoked";

export interface RemoteDevice {
  id: string;
  deviceName: string;
  operatingSystem: string;
  systemVersion: string;
  deviceType: RemoteDeviceType;
  status: RemoteDeviceStatus;
  lastConnectedAtMs?: number | null;
}

export type PairingPhase = "unavailable" | "starting" | "waiting" | "success" | "expired" | "cancelled" | "error";

export type PairingCopyStatus = "idle" | "copying" | "copied" | "failed" | "unavailable";

export interface PairingDisplayState {
  phase: PairingPhase;
  qrImageUrl?: string | null;
  expiresAtMs?: number | null;
  remainingSeconds?: number | null;
  pairedClientName?: string | null;
  errorMessage?: string | null;
}

export type RemoteDeviceTone = "ready" | "neutral" | "danger";

const statusMetadata: Record<RemoteDeviceStatus, { label: string; tone: RemoteDeviceTone }> = {
  online: { label: "在线", tone: "ready" },
  offline: { label: "离线", tone: "neutral" },
  "never-connected": { label: "从未连接", tone: "neutral" },
  revoked: { label: "已撤销", tone: "danger" },
};

export function remoteDeviceStatusMeta(status: RemoteDeviceStatus): { label: string; tone: RemoteDeviceTone } {
  return statusMetadata[status];
}

export function remoteDeviceFromClient(client: RemoteClient): RemoteDevice {
  const status: RemoteDeviceStatus = client.revokedAt != null
    ? "revoked"
    : client.onlineSessionCount > 0
      ? "online"
      : client.lastSeenAt <= client.createdAt
        ? "never-connected"
        : "offline";

  return {
    id: client.credentialId,
    deviceName: client.descriptor.deviceName,
    operatingSystem: client.descriptor.operatingSystem,
    systemVersion: client.descriptor.systemVersion,
    deviceType: remoteDeviceTypeForOperatingSystem(client.descriptor.operatingSystem),
    status,
    lastConnectedAtMs: status === "never-connected" ? null : client.lastSeenAt,
  };
}

export function remoteDeviceTypeForOperatingSystem(operatingSystem: string): RemoteDeviceType {
  const normalized = operatingSystem.trim().toLowerCase();
  if (/ipad|tablet/.test(normalized)) return "tablet";
  if (/iphone|ios|android|phone|mobile/.test(normalized)) return "phone";
  if (/web|browser|chrome|firefox|edge|safari/.test(normalized)) return "browser";
  if (/mac|windows|win32|linux|desktop/.test(normalized)) return "desktop";
  return "unknown";
}

export function remoteDeviceSystemLabel(device: RemoteDevice): string {
  return [device.operatingSystem, device.systemVersion]
    .map((part) => part.trim())
    .filter(Boolean)
    .join(" ") || "未知系统";
}

export function remoteDeviceConnectionLabel(device: RemoteDevice, nowMs = Date.now()): string {
  if (device.status === "never-connected" || device.lastConnectedAtMs == null) {
    return "从未连接";
  }

  const elapsedSeconds = Math.max(0, Math.floor((nowMs - device.lastConnectedAtMs) / 1000));
  if (elapsedSeconds < 60) return "上次连接 刚刚";

  const elapsedMinutes = Math.floor(elapsedSeconds / 60);
  if (elapsedMinutes < 60) return `上次连接 ${elapsedMinutes} 分钟前`;

  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `上次连接 ${elapsedHours} 小时前`;

  const elapsedDays = Math.floor(elapsedHours / 24);
  if (elapsedDays < 30) return `上次连接 ${elapsedDays} 天前`;

  const elapsedMonths = Math.floor(elapsedDays / 30);
  if (elapsedMonths < 12) return `上次连接 ${elapsedMonths} 个月前`;

  return `上次连接 ${Math.floor(elapsedMonths / 12)} 年前`;
}

export function pairingCountdownLabel(remainingSeconds: number): string {
  const safeSeconds = Math.max(0, Math.ceil(remainingSeconds));
  const minutes = Math.floor(safeSeconds / 60);
  const seconds = safeSeconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export function pairingRemainingSeconds(expiresAtMs: number, nowMs = Date.now()): number {
  return Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
}

export function pairingJsonCanBeCopied(display: PairingDisplayState): boolean {
  return display.phase === "waiting" && (display.remainingSeconds ?? 0) > 0;
}

export function pairingPhaseForStatus(
  status: RemotePairingStatusKind,
  remainingSeconds: number,
): PairingPhase {
  if (status === "succeeded") return "success";
  if (status === "expired" || (status === "active" && remainingSeconds <= 0)) return "expired";
  if (status === "cancelled") return "cancelled";
  return "waiting";
}
