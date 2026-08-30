import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const appSource = readFileSync(resolve(__dirname, "App.svelte"), "utf8");
const deviceListSource = readFileSync(resolve(__dirname, "lib/RemoteDeviceList.svelte"), "utf8");
const pairDialogSource = readFileSync(resolve(__dirname, "lib/PairDeviceDialog.svelte"), "utf8");

describe("connections workspace", () => {
  it("renames the navigation and keeps devices before the unchanged runtime controls", () => {
    const connectionStart = appSource.indexOf('{:else if tab === "connections"}');
    const usageStart = appSource.indexOf('{:else if tab === "usage"}', connectionStart);
    const connectionSource = appSource.slice(connectionStart, usageStart);

    expect(appSource).toContain('<Cable size={18} /> 连接');
    expect(connectionSource.indexOf('<h3>设备</h3>')).toBeGreaterThan(-1);
    expect(connectionSource.indexOf('<h3>设备</h3>')).toBeLessThan(connectionSource.indexOf('<h3>本机运行时</h3>'));
    expect(connectionSource).toContain("on:click={refreshRuntimes}");
    expect(connectionSource).toContain("on:click={() => detectRuntime(runtime.providerId)}");
    expect(connectionSource).toContain("on:click={() => chooseRuntimeExecutable(runtime)}");
    expect(connectionSource).toContain("on:click={() => restoreAutomaticRuntime(runtime)}");
  });

  it("starts with bridge-owned device and pairing state empty", () => {
    expect(appSource).toContain("let remoteDevices: RemoteDevice[] = [];");
    expect(appSource).toContain('phase: "unavailable"');
    expect(appSource).not.toMatch(/clientName:\s*["']/);
  });

  it("exposes device status and revocation UI through component inputs", () => {
    expect(deviceListSource).toContain("export let devices: RemoteDevice[] = [];");
    expect(deviceListSource).toContain("export let onRevoke:");
    expect(deviceListSource).toContain("remoteDeviceStatusMeta(device.status)");
    expect(deviceListSource).toContain("撤销访问权限");
  });

  it("renders only a QR image and never a plaintext pairing payload", () => {
    expect(pairDialogSource).toContain("export let display: PairingDisplayState;");
    expect(pairDialogSource).toContain('<img src={display.qrImageUrl} alt="设备配对二维码" />');
    expect(pairDialogSource).toContain('display.phase === "success"');
    expect(pairDialogSource).toContain("pairingCountdownLabel(display.remainingSeconds)");
    expect(pairDialogSource).not.toMatch(/pairing(Code|Credential|Payload)|<pre|<code/);
  });

  it("opens as a native modal and handles both Tab directions inside it", () => {
    expect(pairDialogSource).toContain("<dialog");
    expect(pairDialogSource).toContain("dialogElement.showModal()");
    expect(pairDialogSource).toContain("on:cancel={handleCancel}");
    expect(pairDialogSource).toContain("wrappedDialogFocusIndex(activeIndex, focusableElements.length, event.shiftKey)");
    expect(pairDialogSource).toContain('if (event.key === "Escape")');
    expect(appSource).toContain("addDeviceButton?.focus()");
  });
});
