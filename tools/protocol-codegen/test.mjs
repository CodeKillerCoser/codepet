import assert from "node:assert/strict";
import test from "node:test";

import { generateProtocol, loadProtocolModel } from "./generate.mjs";

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
    ["core-v1", "pet-v1", "provider-v1", "gateway-v1", "gateway-compat-v0"],
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
    "turn.start",
    "approval.resolve",
    "provider.shutdown",
  ]) {
    assert(methods.has(required), `provider manifest is missing ${required}`);
  }
});

test("gateway resources are routed while plugin lifecycle stays private", async () => {
  const model = await loadProtocolModel();
  const gateway = record(model, "gateway-v1");
  const methods = gateway.manifest.methods.map((method) => method.name);
  assert.equal(methods.some((method) => method.startsWith("instance.") || method === "provider.shutdown"), false);
  assert.equal(gateway.manifest.transport.eventCursorField, "eventCursor");
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
});

test("future language generators share the same declared interface", async () => {
  const model = await loadProtocolModel();
  const languages = new Map(model.config.languages.map((language) => [language.id, language]));
  assert.equal(languages.get("rust").status, "active");
  assert.equal(languages.get("typescript").status, "compatibility");
  assert.equal(languages.get("dart").status, "planned");
  assert.equal(languages.get("python").status, "planned");
  assert.equal(new Set(model.config.languages.map((language) => language.interface)).size, 1);
});

test("checked-in SDK files are fresh", async () => {
  await generateProtocol({ checkMode: true });
});
