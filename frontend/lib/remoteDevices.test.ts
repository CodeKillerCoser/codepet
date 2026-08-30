import { describe, expect, it } from "vitest";
import {
  pairingCountdownLabel,
  remoteDeviceConnectionLabel,
  remoteDeviceStatusMeta,
  type RemoteDevice,
} from "./remoteDevices";

function device(overrides: Partial<RemoteDevice> = {}): RemoteDevice {
  return {
    id: "device-one",
    clientName: "CodePet Remote",
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

  it("formats pairing countdown values supplied by the future bridge", () => {
    expect(pairingCountdownLabel(299)).toBe("04:59");
    expect(pairingCountdownLabel(0)).toBe("00:00");
    expect(pairingCountdownLabel(-10)).toBe("00:00");
  });
});
