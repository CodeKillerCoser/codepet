import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  cancelRemotePairing,
  copyRemotePairingJson,
  getRemoteAccessStatus,
  getRemotePairingStatus,
  listRemoteClients,
  retryRemoteAccess,
  revokeRemoteCredential,
  startRemotePairing,
} from "./remoteAccess";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("remote access command bridge", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("uses only the eight dedicated RemoteAccessRuntime commands", async () => {
    const status = {
      phase: "available",
      activeSessionCount: 0,
      pairingAvailable: false,
      diagnostic: null,
    };
    const pairing = {
      pairingId: "pairing-one",
      expiresAt: 2_000,
      qrSvgDataUrl: "data:image/svg+xml;base64,PHN2Zy8+",
    };
    const pairingStatus = { pairingId: "pairing-one", state: "active", expiresAt: 2_000 };
    const clients = [{
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
    }];
    const revoked = { credentialId: "credential-one", revokedAt: 1_500, disconnectedSessionCount: 0 };
    vi.mocked(invoke)
      .mockResolvedValueOnce(status)
      .mockResolvedValueOnce(status)
      .mockResolvedValueOnce(clients)
      .mockResolvedValueOnce(pairing)
      .mockResolvedValueOnce(pairingStatus)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ ...pairingStatus, state: "cancelled" })
      .mockResolvedValueOnce(revoked);

    await expect(getRemoteAccessStatus()).resolves.toEqual(status);
    await expect(retryRemoteAccess()).resolves.toEqual(status);
    await expect(listRemoteClients()).resolves.toEqual(clients);
    await expect(startRemotePairing()).resolves.toEqual(pairing);
    await expect(getRemotePairingStatus("pairing-one")).resolves.toEqual(pairingStatus);
    await expect(copyRemotePairingJson("pairing-one")).resolves.toBeUndefined();
    await expect(cancelRemotePairing("pairing-one")).resolves.toMatchObject({ state: "cancelled" });
    await expect(revokeRemoteCredential("credential-one")).resolves.toEqual(revoked);

    expect(invoke).toHaveBeenNthCalledWith(1, "remote_access_status");
    expect(invoke).toHaveBeenNthCalledWith(2, "retry_remote_access");
    expect(invoke).toHaveBeenNthCalledWith(3, "list_remote_clients");
    expect(invoke).toHaveBeenNthCalledWith(4, "start_remote_pairing");
    expect(invoke).toHaveBeenNthCalledWith(5, "get_remote_pairing_status", { pairingId: "pairing-one" });
    expect(invoke).toHaveBeenNthCalledWith(6, "copy_remote_pairing_json", { pairingId: "pairing-one" });
    expect(invoke).toHaveBeenNthCalledWith(7, "cancel_remote_pairing", { pairingId: "pairing-one" });
    expect(invoke).toHaveBeenNthCalledWith(8, "revoke_remote_credential", { credentialId: "credential-one" });
  });

  it("keeps clipboard errors inside the native copy command boundary", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({
      code: "remote_pairing_clipboard_write_failed",
      message: "clipboard unavailable",
      retryable: true,
    });

    await expect(copyRemotePairingJson("pairing-one")).rejects.toMatchObject({
      code: "remote_pairing_clipboard_write_failed",
    });
    expect(invoke).toHaveBeenCalledWith("copy_remote_pairing_json", { pairingId: "pairing-one" });
  });

  it("keeps delayed cleanup bound to the pairing id that created it", async () => {
    let finishOldCleanup: ((value: { pairingId: string; state: string; expiresAt: number }) => void) | null = null;
    const oldCleanupResult = new Promise<{ pairingId: string; state: string; expiresAt: number }>((resolve) => {
      finishOldCleanup = resolve;
    });
    const nextPairing = {
      pairingId: "pairing-new",
      expiresAt: 3_000,
      qrSvgDataUrl: "data:image/svg+xml;base64,PHN2Zy8+",
    };
    vi.mocked(invoke)
      .mockReturnValueOnce(oldCleanupResult)
      .mockResolvedValueOnce(nextPairing);

    const delayedCleanup = cancelRemotePairing("pairing-old");
    await expect(startRemotePairing()).resolves.toEqual(nextPairing);
    finishOldCleanup?.({ pairingId: "pairing-old", state: "cancelled", expiresAt: 2_000 });
    await expect(delayedCleanup).resolves.toMatchObject({ pairingId: "pairing-old", state: "cancelled" });

    expect(invoke).toHaveBeenNthCalledWith(1, "cancel_remote_pairing", { pairingId: "pairing-old" });
    expect(invoke).toHaveBeenNthCalledWith(2, "start_remote_pairing");
    expect(invoke).not.toHaveBeenCalledWith("cancel_remote_pairing", { pairingId: "pairing-new" });
  });
});
