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
    expect(connectionSource).toContain("<dt>用户选择</dt>");
    expect(connectionSource).toContain('runtime.configuredExecutable ?? "自动选择"');
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
    expect(deviceListSource).toContain("{device.deviceName}");
    expect(deviceListSource).toContain("remoteDeviceSystemLabel(device)");
    expect(deviceListSource).toContain("撤销访问权限");
    expect(appSource).toContain("await revokeRemoteCredential(device.id)");
    expect(appSource).toContain("applyRemoteClientSnapshot(await listRemoteClients())");
  });

  it("polls incoming pairing requests and requires matching-code confirmation", () => {
    expect(appSource).toContain("listRemotePairingRequests()");
    expect(appSource).toContain("request.confirmationCode");
    expect(appSource).toContain("请确认 Remote 上显示相同配对码");
    expect(appSource).toContain("resolveRemotePairingRequest(request.requestId, accepted)");
  });

  it("isolates Remote Host failures and provides a retry without changing runtime state", () => {
    expect(appSource).toContain("remoteRuntimeStatus = await getRemoteAccessStatus()");
    expect(appSource).toContain("remoteRuntimeStatus = await retryRemoteAccess()");
    expect(appSource).toContain("设备管理暂不可用，其他设置不受影响。");
    expect(appSource).toContain("remoteCommandError = remoteCommandDiagnostic");
    expect(appSource).not.toContain("agentRuntimes = await listRemoteClients");
    expect(appSource).not.toContain("events = await listRemoteClients");
  });

  it("keeps pairing JSON behind an active-only copy control and never renders it as text", () => {
    expect(pairDialogSource).toContain("export let display: PairingDisplayState;");
    expect(pairDialogSource).toContain('<img src={display.qrImageUrl} alt="设备配对二维码" />');
    expect(pairDialogSource).toContain('display.phase === "success"');
    expect(pairDialogSource).toContain("pairingCountdownLabel(display.remainingSeconds)");
    expect(pairDialogSource).toContain('display.phase === "cancelled"');
    expect(pairDialogSource).toContain('display.phase === "error"');
    expect(appSource).toContain("getRemotePairingStatus(pairingId)");
    expect(appSource).toContain("status.qrSvgDataUrl ?? null");
    expect(appSource).toContain("copyRemotePairingJson(pairingId)");
    expect(appSource).toContain("pairingCopyRequestIsCurrent(requestToken, pairingId)");
    expect(appSource).toContain("await cancelRemotePairing(pairingId)");
    expect(appSource).toContain('phase: "success"');
    expect(pairDialogSource).toContain("复制配对 JSON");
    expect(pairDialogSource).toContain('copyStatus === "unavailable"');
    expect(pairDialogSource).toContain("disabled={!canCopyPairingJson || copyStatus === \"copying\" || copyStatus === \"unavailable\"}");
    expect(pairDialogSource).toContain("配对 JSON 已过期");
    expect(pairDialogSource).toContain('copyStatus === "failed" || copyStatus === "unavailable" ? "alert" : "status"');
    expect(pairDialogSource).not.toMatch(/pairing(Code|Credential|Payload)|<pre|<code/);
    expect(pairDialogSource).not.toContain("{pairingJson}");
    expect(appSource).toContain("qrImageUrl: null");
  });

  it("opens as a native modal and handles both Tab directions inside it", () => {
    expect(pairDialogSource).toContain("<dialog");
    expect(pairDialogSource).toContain("dialogElement.showModal()");
    expect(pairDialogSource).toContain("on:cancel={handleCancel}");
    expect(pairDialogSource).toContain("wrappedDialogFocusIndex(activeIndex, focusableElements.length, event.shiftKey)");
    expect(pairDialogSource).toContain('if (event.key === "Escape")');
    expect(appSource).toContain("addDeviceButton?.focus()");
  });

  it("binds asynchronous cleanup to the old pairing id instead of mutable active state", () => {
    const closeStart = appSource.indexOf("async function closePairDeviceDialog()");
    const closeEnd = appSource.indexOf("function pairingCancellationAlreadyTerminal", closeStart);
    const closeSource = appSource.slice(closeStart, closeEnd);

    expect(closeSource).toContain("const pairingId = activePairingId;");
    expect(closeSource).toContain("await cancelRemotePairing(pairingId)");
    expect(closeSource).not.toContain("cancelRemotePairing(activePairingId)");
    expect(appSource).toContain("cancelRemotePairing(started.pairingId)");
    expect(appSource).not.toMatch(/cancelRemotePairing\(\s*\)/);
  });
});
