import type {
  CurrentCredentialDeleteResponse,
  ConversationGetResponse,
  ConversationSearchRequest,
  ConversationSearchResponse,
  HandshakeRequest,
  HandshakeResponse,
  PairingExchangeRequest,
  PairingExchangeResponse,
  PairingQrPayload,
  TurnOutputDeltaEvent,
} from "../src/generated";

const client: HandshakeRequest = {
  clientId: "remote-client-phone-1",
  device: {
    deviceName: "Alice's Pixel",
    operatingSystem: "Android",
    systemVersion: "16",
  },
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
  device: client.device,
};

const handshake: HandshakeResponse = {
  selectedVersion: 1,
  serverName: "CodePet Gateway",
  serverVersion: "0.1.4",
  device: {
    deviceId: qr.hostDeviceId,
    descriptor: {
      deviceName: qr.displayName,
      operatingSystem: "macOS",
      systemVersion: "15.6",
    },
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

const searchRequest: ConversationSearchRequest = {
  route: {
    deviceId: "device-macbook-1",
    providerPluginId: "dev.codepet.codex",
    providerInstanceId: "codex-work",
  },
  searchTerm: "gateway protocol",
  cursor: "search-cursor",
  limit: 20,
};

const searchResponse: ConversationSearchResponse = {
  conversations: [],
  pageInfo: { nextCursor: "search-next" },
  snapshotCursor: "event-0",
};

const history: ConversationGetResponse["items"] = [
  {
    resource: {
      deviceId: "device-macbook-1",
      providerPluginId: "dev.codepet.codex",
      providerInstanceId: "codex-work",
      nativeResourceId: "message-agent-01",
    },
    turn: {
      deviceId: "device-macbook-1",
      providerPluginId: "dev.codepet.codex",
      providerInstanceId: "codex-work",
      nativeResourceId: "turn-01",
    },
    conversation: {
      deviceId: "device-macbook-1",
      providerPluginId: "dev.codepet.codex",
      providerInstanceId: "codex-work",
      nativeResourceId: "thread-01",
    },
    kind: "message",
    status: "completed",
    role: "assistant",
    contents: [
      {
        contentId: "message-agent-01:text",
        kind: "text",
        text: "Committed assistant text",
      },
    ],
  },
];

const delta: TurnOutputDeltaEvent = {
  turn: history[0].turn,
  conversation: history[0].conversation,
  itemId: history[0].resource.nativeResourceId,
  contentId: history[0].contents[0].contentId,
  kind: history[0].contents[0].kind,
  delta: " delta",
};

void [exchange, exchangeResponse, deleted, searchRequest, searchResponse, history, delta];
