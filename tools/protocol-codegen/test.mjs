import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  GENERATOR_TARGET_INTERFACE,
  buildProtocolIr,
  configureProtocolGenerator,
  generateProtocol,
  generatorTargetRegistry,
  loadProtocolModel,
  repositoryRoot,
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

test("runtime inventory always returns harnessList and nullable selected", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1");
  const inventory = provider.schema.$defs.RuntimeGetInstalledResponse;
  assert.deepEqual(inventory.required, ["harnessList", "selected"]);
  assert.equal(inventory.properties.harnessList.items.$ref, "#/$defs/RuntimeInstallation");
  assert.equal(inventory.properties.installed, undefined);
  assert.deepEqual(inventory.properties.selected.oneOf, [
    { $ref: "#/$defs/RuntimeInstallation" }, { type: "null" },
  ]);
});

test("RTC framing is additive to Gateway WSS and rejects unknown profiles", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  assert.equal(gateway.manifest.transport.framing, "websocket-text");
  assert.deepEqual(gateway.manifest.transport.additionalFramings, ["webrtc-cpg1"]);
  validateManifest(gateway, model);
  for (const invalid of ["webrtc-cpg1", ["unknown"], ["webrtc-cpg1", "webrtc-cpg1"]]) {
    gateway.manifest.transport.additionalFramings = invalid;
    assert.throws(() => validateManifest(gateway, model), /additionalFramings/);
  }
});

test("recent v1 preserves list and read boundaries with capability-gated atomic queries", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1");
  const gateway = record(model, "gateway-v1");
  assert.equal(provider.manifest.version, 1);
  assert.equal(gateway.manifest.version, 1);
  for (const name of ["conversation.active.list", "conversation.unread.list", "conversation.markRead"]) {
    assert.equal(provider.manifest.methods.find((method) => method.name === name)?.capability, name);
  }
  assert.equal(gateway.manifest.methods.find((method) => method.name === "conversation.recent")?.capability, "conversation.recent");
  const pd = provider.schema.$defs;
  const gd = gateway.schema.$defs;
  assert.deepEqual(pd.ConversationListRequest.required, ["route", "projectFilter"]);
  assert.deepEqual(gd.ConversationListRequest.required, ["providerId", "projectFilter"]);
  assert.equal(gd.ConversationListRequest.properties.query, undefined);
  assert.equal(gd.ConversationRecentRequest.properties.readerScope, undefined);
  assert.equal(gd.ConversationRecentRequest.properties.projectFilter, undefined);
  assert.deepEqual(gd.ConversationMarkReadRequest.required, ["conversation", "observedActivityVersion"]);
  assert.equal(gd.ConversationMarkReadRequest.properties.readerScope, undefined);
  assert(pd.ConversationUnreadListRequest.required.includes("readerScope"));
  assert(pd.ConversationMarkReadRequest.required.includes("readerScope"));
  assert.deepEqual(gd.ConversationRecentResponse.required, ["conversations", "pageInfo", "revision", "snapshotCursor"]);
  assert.notEqual(gd.ConversationRecentResponse.properties.revision.$ref, gd.ConversationRecentResponse.properties.snapshotCursor.$ref);
  for (const kind of ["Active", "Unread"]) {
    assert.equal(pd[`Conversation${kind}ListResponse`].properties.revision.$ref, "#/$defs/ConversationEnumerationRevision");
    assert.equal(pd[`Conversation${kind}ChangedEvent`].properties.revision.type, "string");
    assert.equal(pd[`Conversation${kind}ChangedEvent`].properties.revision.$ref, undefined);
  }
  assert.deepEqual(pd.ConversationListQuery.oneOf, [
    { $ref: "#/$defs/ConversationUpdatedAfterQuery" },
    { $ref: "#/$defs/ConversationIdsQuery" },
  ]);
});

test("method namespaces allow atomic submethods but reject empty or malformed segments", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1");
  const method = provider.manifest.methods.find((entry) => entry.name === "conversation.active.list");
  const original = method.name;
  try {
    validateManifest(provider, model);
    for (const name of ["conversation..list", ".conversation.list", "conversation.list.", "conversation.1list"]) {
      method.name = name;
      assert.throws(() => validateManifest(provider, model), /name is invalid/);
    }
  } finally {
    method.name = original;
  }
});

test("schema and method/event manifests are self-consistent", async () => {
  const model = await loadProtocolModel();
  assert.deepEqual(
    model.records.map((entry) => entry.packageConfig.id),
    [
      "core-v1",
      "pet-v1",
      "agent-v1",
      "provider-v1",
      "gateway-v1",
      "channel-lan-v1",
      "desktop-runtime-v0",
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
      /pet-v1 manifest may not reference provider package provider-v1/,
    );
  } finally {
    pet.manifest.error.$ref = original;
  }
});

test("provider is JSON-RPC over stdio and owns plugin instance lifecycle", async () => {
  const model = await loadProtocolModel();
  const provider = record(model, "provider-v1").manifest;
  assert.equal(provider.transport.kind, "json-rpc-2.0");
  assert.equal(provider.transport.framing, "stdio-codepet-mux-v1");
  assert.equal(provider.transport.legacyFraming, undefined);
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
    "conversation.acquireInteraction",
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
    ["provider.ping", "instance.stop", "instance.destroy", "provider.shutdown"],
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
  const providerDefinitions = record(model, "provider-v1").schema.$defs;
  const definitions = record(model, "agent-v1").schema.$defs;
  assert.equal(
    definitions.MessageConversationItem.properties.conversation.$ref,
    "../../core/v1/schema.json#/$defs/RoutedResourceId",
  );
  assert(definitions.MessageConversationItem.required.includes("conversation"));
  assert(providerDefinitions.ConversationGetResponse.required.includes("items"));
  assert.equal(providerDefinitions.ConversationGetResponse.properties.items.items.$ref, "../../agent/v1/schema.json#/$defs/ConversationItem");
  assert.equal(providerDefinitions.ConversationSearchRequest.properties.route.$ref, "#/$defs/ProviderInstanceRoute");
  assert.equal(providerDefinitions.ConversationGetRequest.properties.conversation.$ref, "#/$defs/ProviderResourceId");
  assert.equal(providerDefinitions.ConversationSearchRequest.properties.searchTerm.minLength, 1);
  assert.deepEqual(providerDefinitions.ConversationSearchRequest.required, ["route", "searchTerm"]);
  assert.equal(providerDefinitions.ConversationSearchResponse.properties.pageInfo.$ref, "../../core/v1/schema.json#/$defs/PageInfo");
  assert.equal(definitions.CommandConversationItem.properties.tool.$ref, "#/$defs/ToolInvocation");
  assert.equal(definitions.ConversationItem.oneOf.length, 7);
  for (const variant of definitions.ConversationItem.oneOf) {
    const item = definitions[variant.$ref.split('/').at(-1)];
    assert.equal(item.properties._meta.$ref, "../../core/v1/schema.json#/$defs/JsonObject");
    assert.equal(item.required.includes('_meta'), false);
  }
  assert.deepEqual(definitions.ToolInvocation.required, ["callId", "name", "category", "origin", "input"]);
  assert.equal(definitions.ToolInvocation.properties.input.$ref, "#/$defs/ToolInput");
  assert.equal(definitions.CommandToolInput.properties.truncation.$ref, "#/$defs/ContentTruncation");
  assert.equal(definitions.ToolInvocation.properties.outcome.$ref, "#/$defs/ToolOutcome");
  assert.equal(definitions.ToolInvocation.properties.rawInput, undefined);
  assert.equal(definitions.ToolInvocation.properties.command, undefined);
  assert.equal(definitions.ToolSuccessOutcome.properties.structuredContent, undefined);
  assert.equal(definitions.StructuredJsonContentBlock.required.includes("value"), true);
  assert.deepEqual(definitions.ContentTruncation.required, ["originalBytes", "retainedBytes", "strategy"]);
  assert.equal(definitions.ToolExecutionError.properties.message.maxLength, 512);
  assert.equal(definitions.ToolInvocation.properties.extension, undefined);
  assert.equal(definitions.Conversation.properties.extension, undefined);
  assert(record(model, "provider-v1").manifest.events.some(
    (event) => event.name === "event.conversationItemUpserted",
  ));
});

test("gateway resources are routed while plugin lifecycle stays private", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  const methods = gateway.manifest.methods.map((method) => method.name);
  assert.equal(methods.some((method) => method.startsWith("instance.") || method === "provider.shutdown"), false);
  assert(methods.includes("event.subscribe"));
  assert(methods.includes("conversation.search"));
  assert(methods.includes("conversation.acquireInteraction"));
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
  for (const definition of [
    "ConversationCreateRequest",
    "ConversationListRequest",
    "ConversationSearchRequest",
    "ProjectCreateRequest",
    "ProjectListRequest",
  ]) {
    assert.equal(
      gateway.schema.$defs[definition].properties.providerId.$ref,
      "../../agent/v1/schema.json#/$defs/ProviderId",
    );
    assert(gateway.schema.$defs[definition].required.includes("providerId"));
    assert.equal(gateway.schema.$defs[definition].properties.route, undefined);
  }
  assert.equal(gateway.schema.$defs.GatewayProviderRoute, undefined);
  assert.equal(gateway.schema.$defs.ConversationSearchRequest.properties.searchTerm.minLength, 1);
  assert.deepEqual(gateway.schema.$defs.ConversationSearchRequest.required, ["providerId", "searchTerm"]);
  assert.equal(gateway.schema.$defs.HandshakeResponse.properties.devices, undefined);
  assert.equal(
    gateway.schema.$defs.HandshakeResponse.properties.device.$ref,
    "#/$defs/GatewayDevice",
  );
  assert.equal(gateway.schema.$defs.ProviderSummary.properties.route, undefined);
  assert.equal(gateway.schema.$defs.ProviderSummary.properties.pluginId, undefined);
  assert.deepEqual(gateway.schema.$defs.ProviderIdentity.properties.icon, {
    type: "string",
    pattern: "^https://",
  });
  assert.equal(gateway.schema.$defs.ProviderIdentity.required.includes("icon"), false);
  assert.equal(
    gateway.schema.$defs.ProviderDescribeResponse.properties.capabilities.$ref,
    "#/$defs/GatewayCapabilities",
  );
  assert.equal(
    gateway.schema.$defs.ProviderDescribeResponse.properties.provider.$ref,
    "#/$defs/ProviderSummary",
  );
  assert.deepEqual(Object.keys(record(model, "core-v1").schema.$defs.RoutedResourceId.properties), [
    "providerId",
    "nativeResourceId",
  ]);
  assert(gateway.manifest.events.some((event) => event.name === "provider.changed"));
  assert.equal(gateway.manifest.events.some((event) => event.name === "provider.statusChanged"), false);
  const agentDefinitions = record(model, "agent-v1").schema.$defs;
  for (const definition of ["Conversation", "TurnTask", "Approval"]) {
    assert.equal(
      agentDefinitions[definition].properties.resource.$ref,
      "../../core/v1/schema.json#/$defs/RoutedResourceId",
    );
  }
  assert.equal(
    agentDefinitions.MessageConversationItem.properties.conversation.$ref,
    "../../core/v1/schema.json#/$defs/RoutedResourceId",
  );
  assert(agentDefinitions.MessageConversationItem.required.includes("conversation"));
  assert.equal(agentDefinitions.CommandConversationItem.properties.tool.$ref, "#/$defs/ToolInvocation");
  assert.equal(agentDefinitions.ConversationItem.oneOf.length, 7);
  assert.equal(agentDefinitions.ToolInvocation.properties.input.$ref, "#/$defs/ToolInput");
  assert.equal(agentDefinitions.CommandToolInput.properties.truncation.$ref, "#/$defs/ContentTruncation");
  assert.equal(agentDefinitions.ToolInvocation.properties.outcome.$ref, "#/$defs/ToolOutcome");
  assert.equal(agentDefinitions.ToolInvocation.properties.extension, undefined);
  assert(gateway.manifest.events.some((event) => event.name === "conversation.itemUpserted"));
  assert.deepEqual(
    Object.keys(gateway.schema.$defs.TurnOutputDeltaEvent.properties),
    ["turn", "conversation", "itemId", "contentId", "kind", "delta"],
  );
});

test("provider descriptors and turn controls are explicit discriminated protocol data", async () => {
  const model = await loadProtocolModel();
  const agentDefinitions = record(model, "agent-v1").schema.$defs;
  const providerDefinitions = record(model, "provider-v1").schema.$defs;
  const gatewayTurnSend = record(model, "gateway-v1").manifest.methods.find(
    (method) => method.name === "turn.send",
  );
  assert.equal(gatewayTurnSend.idempotency, "nonIdempotent");
  const instance = providerDefinitions.ProviderInstance;
  assert(instance.required.includes("harness"));
  assert.equal(instance.properties.harness.$ref, "#/$defs/HarnessDescriptor");
  assert(providerDefinitions.ProviderCapabilities.required.includes("revision"));
  assert.equal(providerDefinitions.ProviderCapabilities.properties.turnSend.$ref, "../../agent/v1/schema.json#/$defs/TurnSendCapabilities");
  assert.deepEqual(agentDefinitions.ModelCatalog.oneOf, [
    { $ref: "#/$defs/FlatModelCatalog" },
    { $ref: "#/$defs/GroupedModelCatalog" },
  ]);
  assert.deepEqual(agentDefinitions.ModelSelection.oneOf, [
    { $ref: "#/$defs/FlatModelSelection" },
    { $ref: "#/$defs/GroupedModelSelection" },
  ]);
  const turnResponse = providerDefinitions.TurnStartResponse;
  assert(turnResponse.required.includes("userItem"));
  assert.deepEqual(turnResponse.properties.userItem.oneOf, [
    { $ref: "../../agent/v1/schema.json#/$defs/ConversationItem" },
    { type: "null" },
  ]);
  assert.deepEqual(agentDefinitions.FlatModelSelection.required, ["kind", "modelId"]);
  assert.deepEqual(agentDefinitions.GroupedModelSelection.required, ["kind", "providerId", "modelId"]);
  assert.equal(agentDefinitions.FlatModelSelection.properties.kind.$ref, "#/$defs/FlatModelCatalogKind");
  assert.equal(agentDefinitions.GroupedModelSelection.properties.kind.$ref, "#/$defs/GroupedModelCatalogKind");
  const gatewayDefinitions = record(model, "gateway-v1").schema.$defs;
  assert.deepEqual(gatewayDefinitions.ProviderSummary.required, ["id", "identity", "runtime", "capabilities"]);
  assert.deepEqual(gatewayDefinitions.ProviderCapabilitiesSummary.required, ["revision"]);
  assert.equal(gatewayDefinitions.ProviderRuntime.properties.version.type, "string");
  assert.equal(gatewayDefinitions.ProviderRuntime.properties.executablePath.type, "string");

  const gatewayRust = await readFile("sdk/rust/codepet-gateway-sdk/src/generated.rs", "utf8");
  const providerRust = await readFile("sdk/rust/codepet-provider-sdk/src/generated.rs", "utf8");
  const agentRust = await readFile("sdk/rust/codepet-agent-sdk/src/generated.rs", "utf8");
  assert.match(agentRust, /#\[serde\(untagged\)\]\r?\npub enum ModelCatalog/);
  assert.match(agentRust, /pub kind: FlatModelCatalogKind/);
  assert.match(agentRust, /pub kind: GroupedModelCatalogKind/);
  for (const source of [gatewayRust, providerRust]) {
    assert.match(source, /pub use codepet_agent_sdk::\*;/);
    assert.doesNotMatch(source, /pub enum ModelCatalog/);
    assert.match(source, /pub user_item: Option<ConversationItem>/);
  }
});

test("LAN admission DTOs remain independent from the Gateway JSON-RPC manifest", async () => {
  const model = await loadProtocolModel();
  const channel = record(model, "channel-lan-v1");
  const definitions = channel.schema.$defs;
  assert.deepEqual(channel.packageConfig.publicTypes, [
    "CurrentCredentialDeleteResponse",
    "LanHostIdentity",
    "PairingExchangeRequest",
    "PairingExchangeResponse",
    "PairingRequestCreateRequest",
    "PairingRequestState",
    "PairingRequestStatusResponse",
    "PairingQrPayload",
  ]);
  assert.deepEqual(Object.keys(definitions.LanHostIdentity.properties), [
    "deviceId",
    "descriptor",
    "identityFingerprint",
  ]);
  assert.equal(definitions.LanHostIdentity.properties.descriptor.$ref, "../../../core/v1/schema.json#/$defs/DeviceDescriptor");
  assert.equal(definitions.LanHostIdentity.properties.identityFingerprint.pattern, "^[0-9a-f]{64}$");
  assert.deepEqual(Object.keys(definitions.PairingExchangeRequest.properties), [
    "pairingSecret",
    "clientId",
    "device",
  ]);
  assert.equal(definitions.PairingExchangeRequest.properties.device.$ref, "../../../core/v1/schema.json#/$defs/DeviceDescriptor");
  assert(definitions.PairingExchangeRequest.required.includes("device"));
  assert.equal(definitions.PairingExchangeRequest.properties.pairingSecret["x-codepet-sensitive"], true);
  assert.deepEqual(Object.keys(definitions.PairingExchangeResponse.properties), [
    "device",
    "gatewayUrl",
    "credential",
  ]);
  assert.equal(definitions.PairingExchangeResponse.properties.credential["x-codepet-sensitive"], true);
  assert.deepEqual(definitions.PairingRequestState.enum, [
    "pending",
    "accepted",
    "rejected",
    "expired",
  ]);
  assert.equal(definitions.PairingRequestCreateRequest.properties.clientNonce.pattern, "^[0-9a-f]{64}$");
  assert.equal(definitions.PairingRequestStatusResponse.properties.confirmationCode.pattern, "^[0-9]{6}$");
  assert.equal(definitions.PairingRequestStatusResponse.properties.credential["x-codepet-sensitive"], true);
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
  assert.equal(channel.manifest.methods.length, 0);
  const rust = await readFile("sdk/rust/codepet-lan-channel-sdk/src/generated.rs", "utf8");
  const dart = await readFile("sdk/dart/codepet-lan-channel-sdk/lib/src/generated.dart", "utf8");
  assert.match(rust, /pub const CHANNEL_LAN_SCHEMA_VERSION: u64 = 1;/);
  assert.match(dart, /const int channelLanSchemaVersion = 1;/);
});

test("active generators omit schema definitions outside the public reachability graph", async () => {
  const model = await loadProtocolModel();
  const channel = record(model, "channel-lan-v1");
  channel.schema.$defs.InternalOnly = {
    type: "object",
    properties: {
      value: { type: "string" },
    },
    required: ["value"],
    additionalProperties: false,
  };

  const rust = generatorTargetRegistry.rust.render({ record: channel, model });
  model.protocolIr = buildProtocolIr(model);
  const dart = generatorTargetRegistry.dart.render({ record: channel, model, ir: model.protocolIr });
  assert.doesNotMatch(rust, /InternalOnly/);
  assert.doesNotMatch(dart, /InternalOnly/);
  for (const publicType of channel.packageConfig.publicTypes) {
    assert.match(rust, new RegExp(`pub (?:struct|enum) ${publicType}`));
    assert.match(dart, new RegExp(`(?:final class|enum) ${publicType}`));
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
  const gateway = record(model, "gateway-v1");
  const gatewayIr = model.protocolIr.packagesById.get("gateway-v1");
  assert.deepEqual(
    gatewayIr.service.methods.map((method) => method.name).sort(),
    gateway.manifest.methods.map((method) => method.name).sort(),
  );
  assert.deepEqual(
    gatewayIr.service.events.map((event) => event.name).sort(),
    gateway.manifest.events.map((event) => event.name).sort(),
  );
  assert.equal(
    gatewayIr.service.methods.find((method) => method.name === "turn.send").idempotency,
    "nonIdempotent",
  );
  assert.equal(
    gatewayIr.service.methods.find((method) => method.name === "turn.send").capability,
    "turn.send",
  );
  assert.equal(
    gatewayIr.service.methods.find((method) => method.name === "conversation.acquireInteraction").idempotency,
    "idempotent",
  );

  const first = generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr });
  const second = generatorTargetRegistry.dart.render({ record: gateway, model, ir: model.protocolIr });
  assert.equal(first, second);
  assert.match(first, /import 'package:codepet_core_sdk\/codepet_core_sdk\.dart';/);
  assert.match(first, /import 'package:codepet_agent_sdk\/codepet_agent_sdk\.dart';/);
  assert.match(first, /required Map<String, String> metadata/);
  assert.match(first, /Future<ProjectListResponse> projectList/);
  assert.match(first, /Future<ConversationRecentResponse> conversationRecent\(ConversationRecentRequest request\)/);
  assert.match(first, /conversationRecentChanged\('conversation\.recentChanged', direction: 'gatewayToClient', delivery: 'replayable', scope: 'provider', payloadType: ConversationRecentChangedEvent\)/);
  assert.match(first, /ProtocolIdempotency\.nonIdempotent/);
  assert.match(first, /Future<TurnSendResponse> turnSend\(TurnSendRequest request\)/);
  assert.match(first, /abstract interface class ProtocolClientInstrumentation/);
  assert.match(first, /if \(traceContext != null\) 'meta': traceContext!\.toJson\(\)/);
  const agent = record(model, "agent-v1");
  const agentSource = generatorTargetRegistry.dart.render({ record: agent, model, ir: model.protocolIr });
  assert.match(agentSource, /sealed class ModelCatalog/);
  assert.match(agentSource, /final class FlatModelCatalog extends ModelCatalog/);
  assert.match(agentSource, /if \(peakDaily != null\) 'peakDaily': peakDaily!\.toJson\(\)/);
  assert.doesNotMatch(agentSource, /peakDaily!!/);

  const result = await generateProtocol({ checkMode: true, targets: ["dart"] });
  assert.deepEqual(
    result.generated.map(({ packageId, targetId }) => [packageId, targetId]),
    [["core-v1", "dart"], ["agent-v1", "dart"], ["gateway-v1", "dart"], ["channel-lan-v1", "dart"]],
  );
});

test("Dart adapter rejects ambiguous oneOf before emitting source", async () => {
  const model = await loadProtocolModel();
  const agent = record(model, "agent-v1");
  const union = model.protocolIr.packagesById
    .get("agent-v1")
    .definitions.find((definition) => definition.name === "ModelCatalog");
  const discriminator = union.discriminator;
  union.discriminator = undefined;
  try {
    assert.throws(
      () => generatorTargetRegistry.dart.render({ record: agent, model, ir: model.protocolIr }),
      /untagged or ambiguous oneOf/,
    );
  } finally {
    union.discriminator = discriminator;
  }
});

test("protocol IR rejects ambiguous and open discriminated union variants", async () => {
  const model = await loadProtocolModel();
  const definitions = record(model, "agent-v1").schema.$defs;
  const message = definitions.MessageConversationItem;
  const required = message.required;
  message.required = required.filter((field) => field !== "kind");
  try {
    assert.throws(() => buildProtocolIr(model), /untagged or ambiguous oneOf/);
  } finally {
    message.required = required;
  }

  const additionalProperties = message.additionalProperties;
  message.additionalProperties = true;
  try {
    assert.throws(() => buildProtocolIr(model), /variant MessageConversationItem must be a closed object/);
  } finally {
    message.additionalProperties = additionalProperties;
  }
});

test("TypeScript adapter handles multiple packages and cross-schema imports", async () => {
  const result = await generateProtocol({ checkMode: true, targets: ["typescript"] });
  assert.deepEqual(
    result.generated.map(({ packageId, targetId }) => [packageId, targetId]),
    [
      ["core-v1", "typescript"],
      ["pet-v1", "typescript"],
      ["agent-v1", "typescript"],
      ["provider-v1", "typescript"],
      ["gateway-v1", "typescript"],
      ["desktop-runtime-v0", "typescript"],
    ],
  );
  const source = await readFile("sdk/typescript/codepet-desktop-sdk/src/generated.ts", "utf8");
  assert.match(source, /from "\.\.\/\.\.\/codepet-core-sdk\/src\/generated"/);
  assert.match(source, /import type \{ Cursor, EventSequence, JsonObject, ProtocolError, ProtocolVersion, RequestId, TimestampMs \}/);
  const providerSource = await readFile("sdk/typescript/codepet-provider-sdk/src/generated.ts", "utf8");
  const gatewaySource = await readFile("sdk/typescript/codepet-gateway-sdk/src/generated.ts", "utf8");
  const agentSource = await readFile("sdk/typescript/codepet-agent-sdk/src/generated.ts", "utf8");
  assert.match(agentSource, /export type ConversationItem = MessageConversationItem \| ReasoningConversationItem/);
  assert.match(agentSource, /export type ToolInput = CommandToolInput \| StructuredToolInput \| OpaqueToolInput/);
  assert.match(agentSource, /export type ToolOutcome = ToolSuccessOutcome \| ToolFailureOutcome/);
  for (const jsonRpcSource of [providerSource, gatewaySource]) {
    assert.match(jsonRpcSource, /codepet-agent-sdk\/src\/generated/);
    assert.doesNotMatch(jsonRpcSource, /export type ConversationItem =/);
    assert.match(jsonRpcSource, /jsonrpc: "2\.0"/);
  }
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

test("freshness accepts CRLF checkouts but still rejects content changes", async () => {
  const outputRoot = await mkdtemp(path.join(os.tmpdir(), "codepet-protocol-newlines-"));
  try {
    configureProtocolGenerator({ generatedOutputRoot: outputRoot });
    const result = await generateProtocol();
    const rust = result.generated.find((artifact) => artifact.targetId === "rust");
    const output = path.resolve(outputRoot, rust.output);
    const content = await readFile(output, "utf8");
    assert.match(content.split("\n")[0], /from protocol\/core\/v1\/manifest.json/);
    assert.doesNotMatch(content.split("\n")[0], /\\/);
    await writeFile(output, content.replaceAll("\n", "\r\n"));
    await generateProtocol({ checkMode: true });
    await writeFile(output, `${content}// unexpected content\n`);
    await assert.rejects(generateProtocol({ checkMode: true }), /generated protocol files are stale/);
  } finally {
    configureProtocolGenerator({ generatedOutputRoot: repositoryRoot });
    assert.equal(path.dirname(outputRoot), path.resolve(os.tmpdir()));
    await rm(outputRoot, { recursive: true, force: true });
  }
});
