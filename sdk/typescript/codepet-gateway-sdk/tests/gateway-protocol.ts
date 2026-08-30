import type {
  CurrentCredentialDeleteResponse,
  HandshakeRequest,
  HandshakeResponse,
  PairingExchangeRequest,
  PairingExchangeResponse,
  PairingQrPayload,
} from "../src/generated";

const client: HandshakeRequest = {
  clientId: "remote-client-phone-1",
  clientName: "CodePet Mobile",
  clientVersion: "1.0.0",
  supportedVersions: { minVersion: 1, maxVersion: 1 },
};

const qr: PairingQrPayload = {
  version: 1,
  hostDeviceId: "device-macbook-1",
  displayName: "MacBook",
  httpsBaseUrl: "https://192.168.1.10:49152",
  certSha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  pairingId: "pairing-0123456789abcdef",
  pairingSecret: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
  expiresAt: 1787700300000,
};

const exchange: PairingExchangeRequest = {
  pairingSecret: qr.pairingSecret,
  clientId: client.clientId,
  clientName: client.clientName,
  platform: "android",
};

const handshake: HandshakeResponse = {
  selectedVersion: 1,
  serverName: "CodePet Gateway",
  serverVersion: "0.1.4",
  device: {
    deviceId: qr.hostDeviceId,
    displayName: qr.displayName,
    identityFingerprint: qr.certSha256,
  },
  devices: [],
  providers: [],
  eventCursor: "event-0",
};

const exchangeResponse: PairingExchangeResponse = {
  device: handshake.device,
  gatewayUrl: "wss://192.168.1.10:49152/remote/v1/gateway",
  credential: "opaque-remote-credential",
};

const deleted: CurrentCredentialDeleteResponse = { revoked: true };

void [exchange, exchangeResponse, deleted];
