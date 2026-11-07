import { describe, expect, it } from "vitest";
import { channelLabel, formatBytes } from "./connectionTraffic";
import { remoteDeviceFromClient } from "./remoteDevices";
describe("connection traffic presentation", () => {
  it("keeps unknown routes unknown and scales bytes consistently", () => {
    expect(channelLabel("ipv6-direct")).toBe("IPv6 直连");
    expect(channelLabel("relay")).toBe("中继");
    expect(channelLabel("unknown")).toBe("通道识别中");
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(1024 ** 3)).toBe("1.0 GiB");
  });
  it("preserves actual channel counters in the device projection", () => {
    const connections = [{ channel: "relay", sentBytes: 12, receivedBytes: 34, sentBytesPerSecond: 0, receivedBytesPerSecond: 0 }];
    const device = remoteDeviceFromClient({ credentialId: "c", remoteClientId: "r", descriptor: { deviceName: "Phone", operatingSystem: "Android", systemVersion: "15" }, createdAt: 1, lastSeenAt: 2, onlineSessionCount: 1, connections });
    expect(device.connections).toEqual(connections);
    expect(device.status).toBe("online");
  });
});
