export type RemoteDeviceType = "phone" | "tablet" | "desktop" | "browser" | "unknown";

export type RemoteDeviceStatus = "online" | "offline" | "never-connected" | "revoked";

export interface RemoteDevice {
  id: string;
  clientName: string;
  deviceType: RemoteDeviceType;
  status: RemoteDeviceStatus;
  lastConnectedAtMs?: number | null;
}

export type PairingPhase = "unavailable" | "waiting" | "success" | "expired";

export interface PairingDisplayState {
  phase: PairingPhase;
  qrImageUrl?: string | null;
  expiresAtMs?: number | null;
  remainingSeconds?: number | null;
  pairedClientName?: string | null;
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
