import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  GENERATOR_TARGET_INTERFACE,
  buildProtocolIr,
  generateProtocol,
  generatorTargetRegistry,
  loadProtocolModel,
  validateManifest,
  validatePackageReferences,
} from "./generate.mjs";

function record(model, id) {
  const value = model.recordsById.get(id);
  assert(value, `missing protocol package ${id}`);
  return value;
}

function allRefs(value, refs = []) {
  if (!value || typeof value !== "object") return refs;
  if (typeof value.$ref === "string") refs.push(value.$ref);
  for (const child of Object.values(value)) {
    if (Array.isArray(child)) child.forEach((item) => allRefs(item, refs));
    else allRefs(child, refs);
  }
  return refs;
}

test("schema and method/event manifests are self-consistent", async () => {
  const model = await loadProtocolModel();
  assert.deepEqual(
    model.records.map((entry) => entry.packageConfig.id),
    [
      "core-v1",
      "pet-v1",
      "provider-v1",
      "gateway-v1",
      "gateway-v2",
      "channel-lan-v1",
      "gateway-compat-v0",
    ],
  );
});

test("core has no upward dependency and pet never references provider or gateway", async () => {
  const model = await loadProtocolModel();
  assert.deepEqual(record(model, "core-v1").packageConfig.dependencies, []);
  const pet = record(model, "pet-v1");
  assert.deepEqual(pet.packageConfig.dependencies, ["core-v1"]);
  assert.equal(allRefs(pet.schema).some((ref) => ref.includes("provider") || ref.includes("gateway")), false);
  assert.equal(JSON.stringify(pet.schema).includes("Provider"), false);
});

test("manifest references obey the same declared dependency and layer rules as schemas", async () => {
  const model = await loadProtocolModel();
  const pet = record(model, "pet-v1");
  const original = pet.manifest.error.$ref;
  pet.manifest.error.$ref = "../../provider/v1/schema.json#/$defs/ProviderInitializeRequest";
  try {
    assert.throws(
      () => validatePackageReferences(pet, model),
      /pet-v1 manifest may only reference core packages, found provider-v1/,
    );
  } finally {
    pet.manifest.error.$ref = original;
  }
});

test("provider is JSON-RPC over stdio and owns plugin instance lifecycle", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1").manifest;
  assert.equal(provider.transport.kind, "json-rpc-2.0");
  assert.equal(provider.transport.framing, "stdio-json-lines");
  const methods = new Set(provider.methods.map((method) => method.name));
  for (const required of [
    "provider.initialize",
    "provider.describe",
    "instance.create",
    "instance.start",
    "instance.stop",
    "instance.destroy",
    "instance.capabilities",
    "conversation.list",
    "conversation.search",
    "turn.start",
    "approval.resolve",
    "provider.shutdown",
  ]) {
    assert(methods.has(required), `provider manifest is missing ${required}`);
  }
  assert.deepEqual(
    provider.methods
      .filter((method) => method.dispatchLane === "control")
      .map((method) => method.name),
    ["instance.stop", "instance.destroy", "provider.shutdown"],
  );
  const providerIr = model.protocolIr.packagesById.get("provider-v1");
  assert.equal(
    providerIr.service.methods.find((method) => method.name === "instance.stop").dispatchLane,
    "control",
  );
  assert.equal(
    providerIr.service.methods.find((method) => method.name === "conversation.list").dispatchLane,
    "normal",
  );
});

test("provider history items carry routed conversation ownership", async () => {
  const model = await loadProtocolModel();
  const definitions = record(model, "provider-v1").schema.$defs;
  assert.equal(
    definitions.ConversationItem.properties.conversation.$ref,
    "../../core/v1/schema.json#/$defs/RoutedResourceId",
  );
  assert(definitions.ConversationItem.required.includes("conversation"));
  assert(definitions.ConversationGetResponse.required.includes("items"));
  assert.equal(definitions.ConversationSearchRequest.properties.route.$ref, "#/$defs/ProviderInstanceRoute");
  assert.equal(definitions.ConversationSearchRequest.properties.searchTerm.minLength, 1);
  assert.deepEqual(definitions.ConversationSearchRequest.required, ["route", "searchTerm"]);
  assert.equal(definitions.ConversationSearchResponse.properties.pageInfo.$ref, "../../core/v1/schema.json#/$defs/PageInfo");
});

test("gateway resources are routed while plugin lifecycle stays private", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  const methods = gateway.manifest.methods.map((method) => method.name);
  assert.equal(methods.some((method) => method.startsWith("instance.") || method === "provider.shutdown"), false);
  assert(methods.includes("event.subscribe"));
  assert(methods.includes("conversation.search"));
  assert.equal(gateway.manifest.transport.eventCursorField, "eventCursor");
  assert.equal(
    gateway.schema.$defs.EventSubscribeRequest.properties.afterCursor.$ref,
    "../../core/v1/schema.json#/$defs/EventCursor",
  );
  assert.equal(
    gateway.schema.$defs.EventSubscribeResponse.properties.subscribedAfterCursor.$ref,
    "../../core/v1/schema.json#/$defs/EventCursor",
  );
  assert.equal(gateway.schema.$defs.ConversationListResponse.properties.eventCursor, undefined);
  for (const definition of ["ConversationListResponse", "ConversationSearchResponse", "ConversationGetResponse"]) {
    assert.equal(
      gateway.schema.$defs[definition].properties.snapshotCursor.$ref,
      "../../core/v1/schema.json#/$defs/EventCursor",
    );
    assert(gateway.schema.$defs[definition].required.includes("snapshotCursor"));
  }
  assert.equal(gateway.schema.$defs.ConversationSearchRequest.properties.route.$ref, "#/$defs/GatewayProviderRoute");
  assert.equal(gateway.schema.$defs.ConversationSearchRequest.properties.searchTerm.minLength, 1);
  assert.deepEqual(gateway.schema.$defs.ConversationSearchRequest.required, ["route", "searchTerm"]);
  assert.equal(
    gateway.schema.$defs.ProviderInstance.properties.pluginId.$ref,
    "../../core/v1/schema.json#/$defs/ProviderPluginId",
  );
  for (const definition of ["Conversation", "TurnTask", "Approval"]) {
    assert.equal(
      gateway.schema.$defs[definition].properties.resource.$ref,
      "../../core/v1/schema.json#/$defs/RoutedResourceId",
    );
  }
  assert.equal(
    gateway.schema.$defs.ConversationItem.properties.conversation.$ref,
    "../../core/v1/schema.json#/$defs/RoutedResourceId",
  );
  assert(gateway.schema.$defs.ConversationItem.required.includes("conversation"));
  assert.deepEqual(
    Object.keys(gateway.schema.$defs.TurnOutputDeltaEvent.properties),
    ["turn", "conversation", "itemId", "contentId", "kind", "delta"],
  );
});

test("provider descriptors and turn controls are explicit discriminated protocol data", async () => {
  const model = await loadProtocolModel();
  const gatewayTurnSend = record(model, "gateway-v1").manifest.methods.find(
    (method) => method.name === "turn.send",
  );
  assert.equal(gatewayTurnSend.idempotency, "nonIdempotent");
  for (const packageId of ["provider-v1", "gateway-v1"]) {
    const definitions = record(model, packageId).schema.$defs;
    const instance = definitions.ProviderInstance;
    assert(instance.required.includes("harness"));
    assert.equal(instance.properties.harness.$ref, "#/$defs/HarnessDescriptor");

    const capabilities = packageId === "provider-v1"
      ? definitions.ProviderCapabilities
      : definitions.GatewayCapabilities;
    assert(capabilities.required.includes("revision"));
    assert.equal(capabilities.properties.turnSend.$ref, "#/$defs/TurnSendCapabilities");

    assert.deepEqual(definitions.ModelCatalog.oneOf, [
      { $ref: "#/$defs/FlatModelCatalog" },
      { $ref: "#/$defs/GroupedModelCatalog" },
    ]);
    assert.deepEqual(definitions.ModelSelection.oneOf, [
      { $ref: "#/$defs/FlatModelSelection" },
      { $ref: "#/$defs/GroupedModelSelection" },
    ]);
    const turnResponse = packageId === "provider-v1"
      ? definitions.TurnStartResponse
      : definitions.TurnSendResponse;
    assert(turnResponse.required.includes("userItem"));
    assert.deepEqual(turnResponse.properties.userItem.oneOf, [
      { $ref: "#/$defs/ConversationItem" },
      { type: "null" },
    ]);
    assert.deepEqual(definitions.FlatModelSelection.required, ["kind", "modelId"]);
    assert.deepEqual(definitions.GroupedModelSelection.required, ["kind", "providerId", "modelId"]);
    assert.equal(definitions.FlatModelSelection.properties.kind.$ref, "#/$defs/FlatModelCatalogKind");
    assert.equal(definitions.GroupedModelSelection.properties.kind.$ref, "#/$defs/GroupedModelCatalogKind");
  }

  const gatewayRust = await readFile("sdk/rust/codepet-gateway-sdk/src/generated.rs", "utf8");
  const providerRust = await readFile("sdk/rust/codepet-provider-sdk/src/generated.rs", "utf8");
  for (const source of [gatewayRust, providerRust]) {
    assert.match(source, /#\[serde\(untagged\)\]\npub enum ModelCatalog/);
    assert.match(source, /pub kind: FlatModelCatalogKind/);
    assert.match(source, /pub kind: GroupedModelCatalogKind/);
    assert.match(source, /pub user_item: Option<ConversationItem>/);
  }
  const gatewayTypescript = await readFile(
    "sdk/typescript/codepet-gateway-sdk/src/generated.ts",
    "utf8",
  );
  assert.match(
    gatewayTypescript,
    /export type ModelSelection = FlatModelSelection \| GroupedModelSelection/,
  );
  assert.match(gatewayTypescript, /export interface FlatModelSelection \{\n  kind: FlatModelCatalogKind;/);
  assert.match(gatewayTypescript, /export interface GroupedModelSelection \{\n  kind: GroupedModelCatalogKind;/);
  assert.match(gatewayTypescript, /userItem: ConversationItem \| null/);
});

test("gateway LAN DTOs remain generated types outside the JSON-RPC method manifest", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  const definitions = gateway.schema.$defs;
  assert.deepEqual(gateway.packageConfig.publicTypes, [
    "CurrentCredentialDeleteResponse",
    "PairingExchangeRequest",
    "PairingExchangeResponse",
    "PairingQrPayload",
  ]);
  assert.equal(definitions.HandshakeRequest.properties.clientId.$ref, "../../core/v1/schema.json#/$defs/ClientId");
  assert.equal(definitions.HandshakeRequest.properties.remoteClientId, undefined);
  assert.equal(definitions.HandshakeRequest.properties.clientName, undefined);
  assert.equal(definitions.HandshakeRequest.properties.device.$ref, "#/$defs/DeviceDescriptor");
  assert(definitions.HandshakeRequest.required.includes("device"));
  assert.equal(definitions.HandshakeResponse.properties.device.$ref, "#/$defs/RemoteHostIdentity");
  assert(definitions.HandshakeResponse.required.includes("device"));
  assert.deepEqual(Object.keys(definitions.DeviceDescriptor.properties), [
    "deviceName",
    "operatingSystem",
    "systemVersion",
  ]);
  assert.deepEqual(definitions.DeviceDescriptor.required, [
    "deviceName",
    "operatingSystem",
    "systemVersion",
  ]);
  assert.deepEqual(Object.keys(definitions.RemoteHostIdentity.properties), [
    "deviceId",
    "descriptor",
    "identityFingerprint",
  ]);
  assert.equal(definitions.RemoteHostIdentity.properties.descriptor.$ref, "#/$defs/DeviceDescriptor");
  assert.equal(definitions.RemoteHostIdentity.properties.identityFingerprint.pattern, "^[0-9a-f]{64}$");
  assert.deepEqual(Object.keys(definitions.PairingExchangeRequest.properties), [
    "pairingSecret",
    "clientId",
    "device",
  ]);
  assert.equal(definitions.PairingExchangeRequest.properties.device.$ref, "#/$defs/DeviceDescriptor");
  assert(definitions.PairingExchangeRequest.required.includes("device"));
  assert.equal(definitions.PairingExchangeRequest.properties.pairingSecret["x-codepet-sensitive"], true);
  assert.deepEqual(Object.keys(definitions.PairingExchangeResponse.properties), [
    "device",
    "gatewayUrl",
    "credential",
  ]);
  assert.equal(definitions.PairingExchangeResponse.properties.credential["x-codepet-sensitive"], true);
  assert.deepEqual(Object.keys(definitions.PairingQrPayload.properties), [
    "version",
    "hostDeviceId",
    "displayName",
    "httpsBaseUrl",
    "certSha256",
    "pairingId",
    "pairingSecret",
    "expiresAt",
  ]);
  assert.equal(definitions.PairingQrPayload.properties.certSha256.pattern, "^[0-9a-f]{64}$");
  assert.equal(definitions.PairingQrPayload.properties.pairingSecret["x-codepet-sensitive"], true);
  assert.deepEqual(definitions.CurrentCredentialDeleteResponse.required, ["revoked"]);
  assert.equal(
    gateway.manifest.methods.some((method) => method.name.includes("pairing") || method.name.includes("credential")),
    false,
  );
});

test("active generators omit schema definitions outside the public reachability graph", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  gateway.schema.$defs.InternalOnly = {
    type: "object",
    properties: {
      value: { type: "string" },
    },
    required: ["value"],
    additionalProperties: false,
  };

  const rust = generatorTargetRegistry.rust.render({ record: gateway, model });
  const typescript = generatorTargetRegistry.typescript.render({ record: gateway, model });
  model.protocolIr = buildProtocolIr(model);
  const dart = generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr });
  assert.doesNotMatch(rust, /InternalOnly/);
  assert.doesNotMatch(typescript, /InternalOnly/);
  assert.doesNotMatch(dart, /InternalOnly/);
  for (const publicType of gateway.packageConfig.publicTypes) {
    assert.match(rust, new RegExp(`pub struct ${publicType}`));
    assert.match(typescript, new RegExp(`export interface ${publicType}`));
    assert.match(dart, new RegExp(`final class ${publicType}`));
  }
});

test("future language generators share the same declared interface", async () => {
  const model = await loadProtocolModel();
  const targets = new Map(model.config.targets.map((target) => [target.id, target]));
  assert.equal(targets.get("rust").status, "active");
  assert.equal(targets.get("typescript").status, "active");
  assert.equal(targets.get("dart").status, "active");
  assert.equal(targets.get("python").status, "planned");
  assert.deepEqual(new Set(model.config.targets.map((target) => target.interface)), new Set([GENERATOR_TARGET_INTERFACE]));
});

test("target registry fails closed for fake and planned adapters", async () => {
  await assert.rejects(
    generateProtocol({ checkMode: true, targets: ["fake"] }),
    /unknown generator target: fake/,
  );
  for (const target of ["python"]) {
    await assert.rejects(
      generateProtocol({ checkMode: true, targets: [target] }),
      new RegExp(`generator target is not implemented: ${target}`),
    );
  }
});

test("Dart adapter uses the normalized IR for DTOs, routes, metadata, and package boundaries", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v2");
  const gatewayIr = model.protocolIr.packagesById.get("gateway-v2");
  assert.equal(gatewayIr.service.methods.length, 11);
  assert.equal(gatewayIr.service.events.length, 7);
  assert.equal(
    gatewayIr.service.methods.find((method) => method.name === "turn.send").idempotency,
    "nonIdempotent",
  );
  assert.equal(
    gatewayIr.service.methods.find((method) => method.name === "turn.send").capability,
    "turn.send",
  );

  const first = generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr });
  const second = generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr });
  assert.equal(first, second);
  assert.match(first, /import 'package:codepet_core_sdk\/codepet_core_sdk\.dart';/);
  assert.match(first, /sealed class ModelCatalog/);
  assert.match(first, /final class FlatModelCatalog extends ModelCatalog/);
  assert.match(first, /ProtocolIdempotency\.nonIdempotent/);
  assert.match(first, /Future<TurnSendResponse> turnSend\(TurnSendRequest request\)/);

  const result = await generateProtocol({ checkMode: true, targets: ["dart"] });
  assert.deepEqual(
    result.generated.map(({ packageId, targetId }) => [packageId, targetId]),
    [["core-v1", "dart"], ["gateway-v2", "dart"], ["channel-lan-v1", "dart"]],
  );
});

test("Dart adapter rejects ambiguous oneOf before emitting source", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v2");
  const union = model.protocolIr.packagesById
    .get("gateway-v2")
    .definitions.find((definition) => definition.name === "ModelCatalog");
  const discriminator = union.discriminator;
  union.discriminator = undefined;
  try {
    assert.throws(
      () => generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr }),
      /untagged or ambiguous oneOf/,
    );
  } finally {
    union.discriminator = discriminator;
  }
});

test("TypeScript adapter handles multiple packages and cross-schema imports", async () => {
  const result = await generateProtocol({ checkMode: true, targets: ["typescript"] });
  assert.deepEqual(
    result.generated.map(({ packageId, targetId }) => [packageId, targetId]),
    [["core-v1", "typescript"], ["gateway-v1", "typescript"], ["gateway-compat-v0", "typescript"]],
  );
  const gatewaySource = await readFile("sdk/typescript/codepet-gateway-sdk/src/generated.ts", "utf8");
  assert.match(gatewaySource, /export interface RemoteHostIdentity/);
  assert.match(gatewaySource, /export interface PairingExchangeRequest/);
  assert.match(gatewaySource, /export interface PairingExchangeResponse/);
  assert.match(gatewaySource, /export interface PairingQrPayload/);
  assert.match(gatewaySource, /export interface CurrentCredentialDeleteResponse/);
  const source = await readFile("sdk/typescript/codepet-gateway-sdk/src/compat-v0.ts", "utf8");
  assert.match(source, /from "\.\.\/\.\.\/codepet-core-sdk\/src\/generated"/);
  assert.match(source, /import type \{ Cursor, EventSequence, JsonObject, ProtocolError, ProtocolVersion, RequestId, TimestampMs \}/);
});

test("capability metadata rejects unknown method capability values", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1");
  const method = provider.manifest.methods.find((entry) => entry.name === "conversation.list");
  const original = method.capability;
  method.capability = "conversation.lsit";
  try {
    assert.throws(
      () => validateManifest(provider, model),
      /uses unknown capability conversation\.lsit/,
    );
  } finally {
    method.capability = original;
  }
});

test("checked-in SDK files are fresh", async () => {
  await generateProtocol({ checkMode: true });
});
