import { describe, expect, it } from "vitest";
import {
  pairingCountdownLabel,
  pairingPhaseForStatus,
  pairingRemainingSeconds,
  remoteDeviceConnectionLabel,
  remoteDeviceFromClient,
  remoteDeviceStatusMeta,
  remoteDeviceSystemLabel,
  type RemoteDevice,
} from "./remoteDevices";

function device(overrides: Partial<RemoteDevice> = {}): RemoteDevice {
  return {
    id: "device-one",
    deviceName: "CodePet Remote",
    operatingSystem: "iOS",
    systemVersion: "18.0",
    deviceType: "phone",
    status: "offline",
    lastConnectedAtMs: 1_000_000,
    ...overrides,
  };
}

describe("remote device display state", () => {
  it("maps every device access state to a visible label and tone", () => {
    expect(remoteDeviceStatusMeta("online")).toEqual({ label: "在线", tone: "ready" });
    expect(remoteDeviceStatusMeta("offline")).toEqual({ label: "离线", tone: "neutral" });
    expect(remoteDeviceStatusMeta("never-connected")).toEqual({ label: "从未连接", tone: "neutral" });
    expect(remoteDeviceStatusMeta("revoked")).toEqual({ label: "已撤销", tone: "danger" });
  });

  it("shows relative last-connected time without inventing one for new devices", () => {
    const nowMs = 1_000_000 + 10 * 60 * 1000;

    expect(remoteDeviceConnectionLabel(device(), nowMs)).toBe("上次连接 10 分钟前");
    expect(remoteDeviceConnectionLabel(device({ lastConnectedAtMs: null }), nowMs)).toBe("从未连接");
    expect(remoteDeviceConnectionLabel(device({ status: "never-connected" }), nowMs)).toBe("从未连接");
  });

  it("maps backend client metadata to online, offline, never-connected, and revoked devices", () => {
    const client = {
      credentialId: "credential-one",
      remoteClientId: "client-one",
      descriptor: {
        deviceName: "My Phone",
        operatingSystem: "iOS",
        systemVersion: "18.0",
      },
      createdAt: 1_000,
      lastSeenAt: 1_000,
      revokedAt: null,
      onlineSessionCount: 0,
    };

    expect(remoteDeviceFromClient(client)).toMatchObject({ deviceType: "phone", status: "never-connected", lastConnectedAtMs: null });
    expect(remoteDeviceFromClient({ ...client, lastSeenAt: 2_000 })).toMatchObject({ status: "offline", lastConnectedAtMs: 2_000 });
    expect(remoteDeviceFromClient({ ...client, onlineSessionCount: 2 })).toMatchObject({ status: "online" });
    expect(remoteDeviceFromClient({ ...client, onlineSessionCount: 2, revokedAt: 3_000 })).toMatchObject({ status: "revoked" });
    expect(remoteDeviceFromClient({ ...client, descriptor: { ...client.descriptor, operatingSystem: "macOS" } })).toMatchObject({ deviceType: "desktop", operatingSystem: "macOS" });
    expect(remoteDeviceFromClient({ ...client, descriptor: { ...client.descriptor, operatingSystem: "iPadOS" } })).toMatchObject({ deviceType: "tablet" });
    expect(remoteDeviceSystemLabel(remoteDeviceFromClient(client))).toBe("iOS 18.0");
  });

  it("formats pairing countdown and maps every terminal pairing outcome", () => {
    expect(pairingCountdownLabel(299)).toBe("04:59");
    expect(pairingCountdownLabel(0)).toBe("00:00");
    expect(pairingCountdownLabel(-10)).toBe("00:00");
    expect(pairingRemainingSeconds(301_000, 2_000)).toBe(299);
    expect(pairingPhaseForStatus("active", 10)).toBe("waiting");
    expect(pairingPhaseForStatus("succeeded", 10)).toBe("success");
    expect(pairingPhaseForStatus("expired", 0)).toBe("expired");
    expect(pairingPhaseForStatus("cancelled", 10)).toBe("cancelled");
  });
});
