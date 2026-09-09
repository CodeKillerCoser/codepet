#!/usr/bin/env node

import { readFile, mkdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { generateDart } from "./dart.mjs";

export let repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export let protocolRoot = resolve(repositoryRoot, "protocol");
let configPath = resolve(protocolRoot, "codegen.json");
let outputRoot = repositoryRoot;

export function configureProtocolGenerator({
  sourceRoot = repositoryRoot,
  protocolDirectory = resolve(sourceRoot, "protocol"),
  configFile = resolve(protocolDirectory, "codegen.json"),
  generatedOutputRoot = sourceRoot,
} = {}) {
  repositoryRoot = resolve(sourceRoot);
  protocolRoot = resolve(protocolDirectory);
  configPath = resolve(configFile);
  outputRoot = resolve(generatedOutputRoot);
}

export const GENERATOR_TARGET_INTERFACE = "codepet.protocol.codegen/v1";
const defaultTargetIds = ["rust", "typescript", "dart"];

const supportedKeywords = new Set([
  "$schema",
  "$id",
  "$defs",
  "$ref",
  "title",
  "description",
  "type",
  "enum",
  "oneOf",
  "properties",
  "required",
  "additionalProperties",
  "items",
  "minimum",
  "maximum",
  "minLength",
  "maxLength",
  "minItems",
  "pattern",
  "uniqueItems",
  "x-codepet-sensitive",
]);

function fail(message) {
  throw new Error(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

async function readJson(path) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    fail(`${relative(repositoryRoot, path)}: ${error.message}`);
  }
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactKeys(value, keys, location) {
  assert(isObject(value), `${location} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  assert(JSON.stringify(actual) === JSON.stringify(expected), `${location} keys must be ${expected.join(", ")}`);
}

function parseReference(reference, sourcePath, model, location) {
  assert(typeof reference === "string", `${location} must be a string ref`);
  const match = reference.match(/^(.*)#\/\$defs\/([A-Za-z][A-Za-z0-9]*)$/);
  assert(match, `${location} has an unsupported ref: ${reference}`);
  const targetPath = match[1] ? resolve(dirname(sourcePath), match[1]) : sourcePath;
  const record = model.schemasByPath.get(targetPath);
  assert(record, `${location} references an unregistered schema: ${relative(repositoryRoot, targetPath)}`);
  const target = record.schema.$defs[match[2]];
  assert(target, `${location} cannot resolve ${reference}`);
  return { name: match[2], node: target, record };
}

function referenceTarget(referenceObject, sourcePath, model, location) {
  assert(isObject(referenceObject), `${location} must be an object containing $ref`);
  exactKeys(referenceObject, ["$ref"], location);
  return parseReference(referenceObject.$ref, sourcePath, model, location);
}

function nullableReferenceVariant(node) {
  if (!Array.isArray(node.oneOf) || node.oneOf.length !== 2) return undefined;
  const reference = node.oneOf.find(
    (variant) => isObject(variant) && typeof variant.$ref === "string" && Object.keys(variant).length === 1,
  );
  const nullVariant = node.oneOf.find(
    (variant) => isObject(variant) && variant.type === "null" && Object.keys(variant).length === 1,
  );
  return reference && nullVariant ? reference : undefined;
}

function validateSchemaNode(node, record, model, location) {
  assert(isObject(node), `${location} must be a schema object`);
  for (const keyword of Object.keys(node)) {
    assert(supportedKeywords.has(keyword), `${location} uses unsupported keyword ${keyword}`);
  }

  if (node.$ref !== undefined) {
    exactKeys(node, ["$ref"], location);
    parseReference(node.$ref, record.schemaPath, model, location);
    return;
  }

  if (node.oneOf !== undefined) {
    assert(node.type === undefined && node.enum === undefined, `${location}.oneOf cannot be combined with type or enum`);
    assert(Array.isArray(node.oneOf) && node.oneOf.length >= 2, `${location}.oneOf must contain at least two variants`);
    const nullableReference = nullableReferenceVariant(node);
    if (nullableReference) {
      node.oneOf.forEach((variant, index) => {
        validateSchemaNode(variant, record, model, `${location}.oneOf[${index}]`);
      });
    } else {
      node.oneOf.forEach((variant, index) => {
        referenceTarget(variant, record.schemaPath, model, `${location}.oneOf[${index}]`);
      });
    }
    return;
  }

  if (node.type !== undefined) {
    assert(["object", "array", "string", "integer", "boolean", "null"].includes(node.type), `${location} has unsupported type ${node.type}`);
  }
  if (node.enum !== undefined) {
    assert(node.type === "string", `${location} only supports string enums`);
    assert(Array.isArray(node.enum) && node.enum.length > 0, `${location}.enum must be non-empty`);
    assert(node.enum.every((item) => typeof item === "string"), `${location}.enum must contain strings`);
    assert(new Set(node.enum).size === node.enum.length, `${location}.enum contains duplicates`);
  }
  if (node.type === "object") {
    if (node.properties !== undefined) {
      assert(isObject(node.properties), `${location}.properties must be an object`);
      for (const [name, property] of Object.entries(node.properties)) {
        assert(name === "_meta" || /^[a-z][A-Za-z0-9]*$/.test(name), `${location} property ${name} must be camelCase or reserved _meta`);
        validateSchemaNode(property, record, model, `${location}.properties.${name}`);
      }
    }
    if (node.required !== undefined) {
      assert(Array.isArray(node.required), `${location}.required must be an array`);
      assert(new Set(node.required).size === node.required.length, `${location}.required contains duplicates`);
      for (const name of node.required) {
        assert(node.properties && Object.hasOwn(node.properties, name), `${location}.required references missing property ${name}`);
      }
    }
    if (node.additionalProperties !== undefined) {
      assert(
        typeof node.additionalProperties === "boolean" || isObject(node.additionalProperties),
        `${location}.additionalProperties must be boolean or a schema object`,
      );
      if (isObject(node.additionalProperties)) {
        assert(node.properties === undefined, `${location} cannot combine properties with typed additionalProperties`);
        validateSchemaNode(node.additionalProperties, record, model, `${location}.additionalProperties`);
      }
    }
  }
  if (node.type === "array") {
    assert(node.items !== undefined, `${location}.items is required for arrays`);
    validateSchemaNode(node.items, record, model, `${location}.items`);
  }
  for (const keyword of ["minimum", "maximum", "minLength", "maxLength", "minItems"]) {
    if (node[keyword] !== undefined) {
      assert(Number.isSafeInteger(node[keyword]), `${location}.${keyword} must be a safe integer`);
      if (keyword === "minLength" || keyword === "maxLength" || keyword === "minItems") {
        assert(node[keyword] >= 0, `${location}.${keyword} must be non-negative`);
      }
    }
  }
  if (node.minimum !== undefined && node.maximum !== undefined) {
    assert(node.minimum <= node.maximum, `${location}.minimum exceeds maximum`);
  }
  if (node.minLength !== undefined && node.maxLength !== undefined) {
    assert(node.minLength <= node.maxLength, `${location}.minLength exceeds maxLength`);
  }
  if (node.uniqueItems !== undefined) {
    assert(typeof node.uniqueItems === "boolean", `${location}.uniqueItems must be boolean`);
  }
  if (node.pattern !== undefined) {
    assert(node.type === "string", `${location}.pattern is only supported for strings`);
    assert(typeof node.pattern === "string", `${location}.pattern must be a string`);
    try {
      new RegExp(node.pattern);
    } catch (error) {
      fail(`${location}.pattern is invalid: ${error.message}`);
    }
  }
  if (node["x-codepet-sensitive"] !== undefined) {
    assert(node["x-codepet-sensitive"] === true, `${location}.x-codepet-sensitive must be true`);
  }
}

function visitRefs(node, visit) {
  if (!isObject(node)) return;
  if (typeof node.$ref === "string") visit(node.$ref);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) value.forEach((item) => visitRefs(item, visit));
    else if (isObject(value)) visitRefs(value, visit);
  }
}

function validateSchema(record, model) {
  const { schema, schemaPath, packageConfig } = record;
  assert(isObject(schema), `${packageConfig.id} schema must be an object`);
  assert(schema.$schema === "https://json-schema.org/draft/2020-12/schema", `${packageConfig.id} must use JSON Schema Draft 2020-12`);
  assert(isObject(schema.$defs) && Object.keys(schema.$defs).length > 0, `${packageConfig.id} must define $defs`);
  validateSchemaNode(schema, record, model, `${packageConfig.id}.schema`);
  for (const [name, definition] of Object.entries(schema.$defs)) {
    assert(/^[A-Z][A-Za-z0-9]*$/.test(name), `${packageConfig.id} definition ${name} must be PascalCase`);
    assert(definition.$ref || definition.type || definition.oneOf, `${packageConfig.id} definition ${name} must declare a type, ref, or oneOf`);
    validateSchemaNode(definition, record, model, `${packageConfig.id}.schema.$defs.${name}`);
  }

}

function validateProviderInstanceKinds(record, model) {
  if (record.packageConfig.layer !== "provider") return;
  const definitions = record.schema.$defs;
  const kind = definitions.ProviderInstanceKind;
  assert(kind?.type === "string" && kind.minLength >= 1, `${record.packageConfig.id} ProviderInstanceKind must be a non-empty string`);

  const assertKindField = (definitionName, fieldName, { array = false, nonEmpty = false } = {}) => {
    const definition = definitions[definitionName];
    assert(definition?.type === "object" && definition.properties, `${record.packageConfig.id} is missing ${definitionName}`);
    assert((definition.required ?? []).includes(fieldName), `${record.packageConfig.id} ${definitionName}.${fieldName} must be required`);
    const field = definition.properties[fieldName];
    const item = array ? field?.items : field;
    if (array) {
      assert(field?.type === "array" && field.uniqueItems === true, `${record.packageConfig.id} ${definitionName}.${fieldName} must be a unique array`);
      if (nonEmpty) assert(field.minItems >= 1, `${record.packageConfig.id} ${definitionName}.${fieldName} must be non-empty`);
    }
    const target = referenceTarget(item, record.schemaPath, model, `${record.packageConfig.id}.${definitionName}.${fieldName}`);
    assert(target.record === record && target.name === "ProviderInstanceKind", `${record.packageConfig.id} ${definitionName}.${fieldName} must use ProviderInstanceKind`);
  };

  assertKindField("ProviderPluginDescriptor", "instanceKinds", { array: true, nonEmpty: true });
  assertKindField("InstanceCreateRequest", "instanceKind");
  assertKindField("ProviderInstance", "instanceKind");
}

export function validateManifest(record, model) {
  const { manifest, manifestPath, packageConfig } = record;
  assert(isObject(manifest), `${packageConfig.id} manifest must be an object`);
  assert(manifest.layer === packageConfig.layer, `${packageConfig.id} manifest layer mismatch`);
  assert(manifest.version === packageConfig.version, `${packageConfig.id} manifest version mismatch`);
  assert(["types", "service"].includes(manifest.kind), `${packageConfig.id} manifest kind is invalid`);
  assert(Array.isArray(manifest.methods), `${packageConfig.id} methods must be an array`);
  assert(Array.isArray(manifest.events), `${packageConfig.id} events must be an array`);

  if (manifest.kind === "types") {
    assert(manifest.methods.length === 0 && manifest.events.length === 0, `${packageConfig.id} types manifest cannot declare methods or events`);
    assert(manifest.transport === undefined, `${packageConfig.id} types manifest cannot declare transport`);
    assert(manifest.capabilities === undefined, `${packageConfig.id} types manifest cannot declare capabilities`);
    return;
  }

  validateProviderInstanceKinds(record, model);

  assert(isObject(manifest.transport), `${packageConfig.id} service manifest requires transport`);
  assert(["codepet-envelope", "json-rpc-2.0"].includes(manifest.transport.kind), `${packageConfig.id} has unsupported transport kind`);
  assert(manifest.transport.requestDiscriminator === "method", `${packageConfig.id} request discriminator must be method`);
  assert(typeof manifest.transport.framing === "string" && manifest.transport.framing.length > 0, `${packageConfig.id} transport framing is required`);
  if (manifest.transport.additionalFramings !== undefined) {
    assert(packageConfig.id === "gateway-v1" && manifest.transport.kind === "json-rpc-2.0" && manifest.transport.framing === "websocket-text", `${packageConfig.id} additional framing requires the existing Gateway WSS contract`);
    assert(Array.isArray(manifest.transport.additionalFramings) && manifest.transport.additionalFramings.length === 1 && manifest.transport.additionalFramings[0] === "webrtc-cpg1", `${packageConfig.id} additionalFramings must contain only webrtc-cpg1`);
  }
  const requestId = referenceTarget(manifest.transport.requestId, manifestPath, model, `${packageConfig.id}.transport.requestId`);
  assert(requestId.name === "RequestId", `${packageConfig.id} transport requestId must reference core RequestId`);
  if (manifest.transport.kind === "json-rpc-2.0") {
    assert(manifest.transport.jsonRpcVersion === "2.0", `${packageConfig.id} must use JSON-RPC 2.0`);
    assert(["stdio-json-lines", "stdio-codepet-mux-v1", "websocket-text"].includes(manifest.transport.framing), `${packageConfig.id} JSON-RPC framing must be stdio-json-lines, stdio-codepet-mux-v1, or websocket-text`);
    assert(manifest.transport.eventDiscriminator === "method", `${packageConfig.id} JSON-RPC event discriminator must be method`);
    const rpcError = referenceTarget(manifest.transport.rpcError, manifestPath, model, `${packageConfig.id}.transport.rpcError`);
    assert(rpcError.name === "RpcError", `${packageConfig.id} transport rpcError must reference core RpcError`);
    if (manifest.transport.traceContext !== undefined) {
      exactKeys(manifest.transport.traceContext, ["field", "type"], `${packageConfig.id}.transport.traceContext`);
      assert(/^[a-z][A-Za-z0-9]*$/.test(manifest.transport.traceContext.field), `${packageConfig.id} traceContext field must be camelCase`);
      const traceContext = referenceTarget(manifest.transport.traceContext.type, manifestPath, model, `${packageConfig.id}.transport.traceContext.type`);
      assert(traceContext.name === "TraceContext", `${packageConfig.id} traceContext must reference core TraceContext`);
    }
  } else {
    assert(manifest.transport.responseDiscriminator === "method", `${packageConfig.id} response discriminator must be method`);
    assert(manifest.transport.eventDiscriminator === "event", `${packageConfig.id} event discriminator must be event`);
    assert(/^[a-z][A-Za-z0-9]*$/.test(manifest.transport.versionField), `${packageConfig.id} versionField must be camelCase`);
    const protocolVersion = referenceTarget(manifest.transport.protocolVersion, manifestPath, model, `${packageConfig.id}.transport.protocolVersion`);
    assert(protocolVersion.name === "ProtocolVersion", `${packageConfig.id} transport protocolVersion must reference core ProtocolVersion`);
  }
  if (manifest.transport.eventCursorField !== undefined) {
    assert(/^[a-z][A-Za-z0-9]*$/.test(manifest.transport.eventCursorField), `${packageConfig.id} eventCursorField must be camelCase`);
    referenceTarget(manifest.transport.eventCursor, manifestPath, model, `${packageConfig.id}.transport.eventCursor`);
  }
  referenceTarget(manifest.error, manifestPath, model, `${packageConfig.id}.error`);

  const methodNames = new Set();
  for (const [index, method] of manifest.methods.entries()) {
    const location = `${packageConfig.id}.methods[${index}]`;
    assert(isObject(method), `${location} must be an object`);
    assert(/^[a-z][A-Za-z0-9]*(?:\.[a-z][A-Za-z0-9]*)+$/.test(method.name), `${location}.name is invalid`);
    assert(!methodNames.has(method.name), `${location}.name duplicates ${method.name}`);
    methodNames.add(method.name);
    assert(typeof method.direction === "string" && method.direction.length > 0, `${location}.direction is required`);
    assert(["safe", "idempotent", "nonIdempotent"].includes(method.idempotency), `${location}.idempotency is invalid`);
    assert(["normal", "control"].includes(method.dispatchLane ?? "normal"), `${location}.dispatchLane is invalid`);
    if (method.capability !== undefined) {
      assert(typeof method.capability === "string" && method.capability.length > 0, `${location}.capability must be a non-empty string`);
    }
    for (const side of ["request", "response"]) {
      const target = referenceTarget(method[side], manifestPath, model, `${location}.${side}`);
      assert(target.node.type === "object" && target.node.properties !== undefined, `${location}.${side} must reference an object DTO`);
    }
  }

  const capabilityMethods = manifest.methods.filter((method) => method.capability !== undefined);
  if (capabilityMethods.length === 0) {
    assert(manifest.capabilities === undefined, `${packageConfig.id} declares capability metadata without capability-gated methods`);
  } else {
    exactKeys(manifest.capabilities, ["type", "container", "field"], `${packageConfig.id}.capabilities`);
    assert(/^[a-z][A-Za-z0-9]*$/.test(manifest.capabilities.field), `${packageConfig.id}.capabilities.field must be camelCase`);
    const capabilityType = referenceTarget(manifest.capabilities.type, manifestPath, model, `${packageConfig.id}.capabilities.type`);
    assert(capabilityType.node.type === "string" && Array.isArray(capabilityType.node.enum), `${packageConfig.id} capability type must reference a string enum`);
    const container = referenceTarget(manifest.capabilities.container, manifestPath, model, `${packageConfig.id}.capabilities.container`);
    assert(container.node.type === "object" && container.node.properties, `${packageConfig.id} capability container must reference an object DTO`);
    const field = container.node.properties[manifest.capabilities.field];
    assert(field?.type === "array" && field.uniqueItems === true, `${packageConfig.id} capability field must be a unique array`);
    assert((container.node.required ?? []).includes(manifest.capabilities.field), `${packageConfig.id} capability field must be required`);
    const itemType = referenceTarget(field.items, container.record.schemaPath, model, `${packageConfig.id}.capabilities.container.${manifest.capabilities.field}`);
    assert(itemType.record === capabilityType.record && itemType.name === capabilityType.name, `${packageConfig.id} capability field item type must match capability type`);
    const usedCapabilities = new Set();
    for (const method of capabilityMethods) {
      assert(capabilityType.node.enum.includes(method.capability), `${packageConfig.id} method ${method.name} uses unknown capability ${method.capability}`);
      usedCapabilities.add(method.capability);
    }
    for (const capability of capabilityType.node.enum) {
      assert(usedCapabilities.has(capability), `${packageConfig.id} capability ${capability} is not mapped to a method`);
    }
  }

  const eventNames = new Set();
  for (const [index, event] of manifest.events.entries()) {
    const location = `${packageConfig.id}.events[${index}]`;
    assert(isObject(event), `${location} must be an object`);
    assert(/^[a-z][A-Za-z0-9]*\.[a-z][A-Za-z0-9]*$/.test(event.name), `${location}.name is invalid`);
    assert(!eventNames.has(event.name), `${location}.name duplicates ${event.name}`);
    eventNames.add(event.name);
    assert(typeof event.direction === "string" && event.direction.length > 0, `${location}.direction is required`);
    assert(typeof event.delivery === "string" && event.delivery.length > 0, `${location}.delivery is required`);
    assert(typeof event.scope === "string" && event.scope.length > 0, `${location}.scope is required`);
    const target = referenceTarget(event.payload, manifestPath, model, `${location}.payload`);
    assert(target.node.type === "object" && target.node.properties !== undefined, `${location}.payload must reference an object DTO`);
  }
}

export function validatePackageReferences(record, model) {
  const { packageConfig } = record;
  const allowedDependencyLayers = {
    core: new Set(),
    agent: new Set(["core"]),
    pet: new Set(["core"]),
    provider: new Set(["core", "agent"]),
    gateway: new Set(["core", "agent"]),
    channel: new Set(["core"]),
    desktop: new Set(["core"]),
  };
  const actualDependencies = new Set();
  for (const [sourceName, source, sourcePath] of [
    ["schema", record.schema, record.schemaPath],
    ["manifest", record.manifest, record.manifestPath],
  ]) {
    visitRefs(source, (reference) => {
      const target = parseReference(reference, sourcePath, model, `${packageConfig.id}.${sourceName}`);
      if (target.record.packageConfig.id === packageConfig.id) return;
      const targetPackage = target.record.packageConfig;
      actualDependencies.add(targetPackage.id);
      assert(
        allowedDependencyLayers[packageConfig.layer].has(targetPackage.layer),
        `${packageConfig.id} ${sourceName} may not reference ${targetPackage.layer} package ${targetPackage.id}`,
      );
    });
  }

  const declaredDependencies = new Set(packageConfig.dependencies);
  for (const dependency of actualDependencies) {
    assert(declaredDependencies.has(dependency), `${packageConfig.id} uses undeclared dependency ${dependency}`);
  }
  for (const dependency of declaredDependencies) {
    assert(actualDependencies.has(dependency), `${packageConfig.id} declares unused dependency ${dependency}`);
  }
  return actualDependencies;
}

function validateValue(value, node, record, model, location) {
  if (node.$ref) {
    const target = parseReference(node.$ref, record.schemaPath, model, location);
    return validateValue(value, target.node, target.record, model, location);
  }
  if (node.oneOf) {
    let matches = 0;
    for (const variant of node.oneOf) {
      try {
        validateValue(value, variant, record, model, location);
        matches += 1;
      } catch {
        // A oneOf fixture must match exactly one closed variant.
      }
    }
    assert(matches === 1, `${location} must match exactly one oneOf variant`);
    return;
  }
  if (node.enum) assert(node.enum.includes(value), `${location} must be one of ${node.enum.join(", ")}`);
  if (node.type === "string") {
    assert(typeof value === "string", `${location} must be a string`);
    if (node.minLength !== undefined) assert(value.length >= node.minLength, `${location} is too short`);
    if (node.maxLength !== undefined) assert(value.length <= node.maxLength, `${location} is too long`);
    if (node.pattern !== undefined) assert(new RegExp(node.pattern).test(value), `${location} does not match ${node.pattern}`);
  } else if (node.type === "integer") {
    assert(Number.isSafeInteger(value), `${location} must be a safe integer`);
    if (node.minimum !== undefined) assert(value >= node.minimum, `${location} is below minimum`);
    if (node.maximum !== undefined) assert(value <= node.maximum, `${location} is above maximum`);
  } else if (node.type === "boolean") {
    assert(typeof value === "boolean", `${location} must be boolean`);
  } else if (node.type === "null") {
    assert(value === null, `${location} must be null`);
  } else if (node.type === "array") {
    assert(Array.isArray(value), `${location} must be an array`);
    if (node.minItems !== undefined) assert(value.length >= node.minItems, `${location} has too few items`);
    value.forEach((item, index) => validateValue(item, node.items, record, model, `${location}[${index}]`));
    if (node.uniqueItems) assert(new Set(value.map((item) => JSON.stringify(item))).size === value.length, `${location} contains duplicate items`);
  } else if (node.type === "object") {
    assert(isObject(value), `${location} must be an object`);
    for (const required of node.required ?? []) assert(Object.hasOwn(value, required), `${location}.${required} is required`);
    for (const [name, item] of Object.entries(value)) {
      if (node.properties && Object.hasOwn(node.properties, name)) {
        validateValue(item, node.properties[name], record, model, `${location}.${name}`);
      } else {
        assert(node.additionalProperties !== false, `${location}.${name} is not allowed`);
        if (isObject(node.additionalProperties)) {
          validateValue(item, node.additionalProperties, record, model, `${location}.${name}`);
        }
      }
    }
  }
}

async function validateFixtures(record, model) {
  const { manifest, manifestPath, packageConfig } = record;
  if (!manifest.fixtures) return;
  const fixtureIndexPath = resolve(dirname(manifestPath), manifest.fixtures);
  const fixtures = await readJson(fixtureIndexPath);
  assert(Array.isArray(fixtures) && fixtures.length > 0, `${packageConfig.id} fixture index must be non-empty`);
  for (const fixture of fixtures) {
    assert(isObject(fixture) && typeof fixture.file === "string" && typeof fixture.kind === "string" && typeof fixture.name === "string", `${packageConfig.id} fixture entry is invalid`);
    assert(fixture.valid === undefined || typeof fixture.valid === "boolean", `${packageConfig.id} fixture entry valid must be boolean`);
    const value = await readJson(resolve(dirname(fixtureIndexPath), fixture.file));
    const location = `${packageConfig.id} fixture ${fixture.file}`;
    if (fixture.kind === "type") {
      const target = record.schema.$defs[fixture.name];
      assert(target, `${location} references unknown type ${fixture.name}`);
      if (fixture.valid === false) {
        let rejected = false;
        try {
          validateValue(value, target, record, model, location);
        } catch {
          rejected = true;
        }
        assert(rejected, `${location} is marked invalid but matches ${fixture.name}`);
      } else {
        validateValue(value, target, record, model, location);
      }
      continue;
    }
    assert(fixture.valid !== false, `${location} negative fixtures currently require kind type`);
    const method = manifest.methods.find((entry) => entry.name === fixture.name);
    const event = manifest.events.find((entry) => entry.name === fixture.name);
    const requestId = referenceTarget(manifest.transport.requestId, manifestPath, model, `${location}.id`);

    if (manifest.transport.kind === "codepet-envelope") {
      assert(value[manifest.transport.versionField] === manifest.version, `${location}.${manifest.transport.versionField} must equal manifest version`);
      if (fixture.kind === "request") {
        assert(method, `${location} references unknown method ${fixture.name}`);
        exactKeys(value, [manifest.transport.versionField, "id", manifest.transport.requestDiscriminator, "params"], location);
        assert(value[manifest.transport.requestDiscriminator] === fixture.name, `${location}.${manifest.transport.requestDiscriminator} is invalid`);
        validateValue(value.id, requestId.node, requestId.record, model, `${location}.id`);
        const target = referenceTarget(method.request, manifestPath, model, `${location}.params`);
        validateValue(value.params, target.node, target.record, model, `${location}.params`);
      } else if (fixture.kind === "response") {
        assert(method, `${location} references unknown method ${fixture.name}`);
        exactKeys(value, [manifest.transport.versionField, "id", manifest.transport.responseDiscriminator, "response"], location);
        assert(value[manifest.transport.responseDiscriminator] === fixture.name, `${location}.${manifest.transport.responseDiscriminator} is invalid`);
        validateValue(value.id, requestId.node, requestId.record, model, `${location}.id`);
        assert(isObject(value.response), `${location}.response must be an object`);
        if (value.response.status === "ok") {
          exactKeys(value.response, ["status", "result"], `${location}.response`);
          const target = referenceTarget(method.response, manifestPath, model, `${location}.response.result`);
          validateValue(value.response.result, target.node, target.record, model, `${location}.response.result`);
        } else {
          assert(value.response.status === "error", `${location}.response.status is invalid`);
          exactKeys(value.response, ["status", "error"], `${location}.response`);
          const target = referenceTarget(manifest.error, manifestPath, model, `${location}.response.error`);
          validateValue(value.response.error, target.node, target.record, model, `${location}.response.error`);
        }
      } else if (fixture.kind === "event") {
        assert(event, `${location} references unknown event ${fixture.name}`);
        const keys = [manifest.transport.versionField, manifest.transport.eventDiscriminator, "payload"];
        if (manifest.transport.eventCursorField) keys.push(manifest.transport.eventCursorField);
        exactKeys(value, keys, location);
        assert(value[manifest.transport.eventDiscriminator] === fixture.name, `${location}.${manifest.transport.eventDiscriminator} is invalid`);
        if (manifest.transport.eventCursorField) {
          const target = referenceTarget(manifest.transport.eventCursor, manifestPath, model, `${location}.${manifest.transport.eventCursorField}`);
          validateValue(value[manifest.transport.eventCursorField], target.node, target.record, model, `${location}.${manifest.transport.eventCursorField}`);
        }
        const target = referenceTarget(event.payload, manifestPath, model, `${location}.payload`);
        validateValue(value.payload, target.node, target.record, model, `${location}.payload`);
      } else {
        fail(`${location} has unsupported kind ${fixture.kind}`);
      }
    } else {
      assert(value.jsonrpc === "2.0", `${location}.jsonrpc must be 2.0`);
      if (fixture.kind === "request") {
        assert(method, `${location} references unknown method ${fixture.name}`);
        exactKeys(value, ["jsonrpc", "id", "method", "params"], location);
        assert(value.method === fixture.name, `${location}.method is invalid`);
        validateValue(value.id, requestId.node, requestId.record, model, `${location}.id`);
        const target = referenceTarget(method.request, manifestPath, model, `${location}.params`);
        validateValue(value.params, target.node, target.record, model, `${location}.params`);
      } else if (fixture.kind === "response") {
        assert(method, `${location} references unknown method ${fixture.name}`);
        assert(Object.hasOwn(value, "result") !== Object.hasOwn(value, "error"), `${location} must contain exactly one of result or error`);
        validateValue(value.id, requestId.node, requestId.record, model, `${location}.id`);
        if (Object.hasOwn(value, "result")) {
          exactKeys(value, ["jsonrpc", "id", "result"], location);
          const target = referenceTarget(method.response, manifestPath, model, `${location}.result`);
          validateValue(value.result, target.node, target.record, model, `${location}.result`);
        } else {
          exactKeys(value, ["jsonrpc", "id", "error"], location);
          const target = referenceTarget(manifest.transport.rpcError, manifestPath, model, `${location}.error`);
          validateValue(value.error, target.node, target.record, model, `${location}.error`);
        }
      } else if (fixture.kind === "event") {
        assert(event, `${location} references unknown event ${fixture.name}`);
        exactKeys(value, ["jsonrpc", "method", "params"], location);
        assert(value.method === fixture.name, `${location}.method is invalid`);
        const target = referenceTarget(event.payload, manifestPath, model, `${location}.params.payload`);
        if (manifest.transport.eventCursorField) {
          assert(isObject(value.params), `${location}.params must be an object`);
          exactKeys(value.params, [manifest.transport.eventCursorField, "payload"], `${location}.params`);
          const cursor = referenceTarget(manifest.transport.eventCursor, manifestPath, model, `${location}.params.${manifest.transport.eventCursorField}`);
          validateValue(value.params[manifest.transport.eventCursorField], cursor.node, cursor.record, model, `${location}.params.${manifest.transport.eventCursorField}`);
          validateValue(value.params.payload, target.node, target.record, model, `${location}.params.payload`);
        } else {
          validateValue(value.params, target.node, target.record, model, `${location}.params`);
        }
      } else {
        fail(`${location} has unsupported kind ${fixture.kind}`);
      }
    }
  }
}

export async function loadProtocolModel({ config: suppliedConfig } = {}) {
  const config = suppliedConfig ?? await readJson(configPath);
  assert(config.generatorInterfaceVersion === 1, "protocol/codegen.json has unsupported generatorInterfaceVersion");
  assert(Array.isArray(config.targets) && config.targets.length >= 4, "protocol/codegen.json must declare generator targets");
  const targetIds = new Set(config.targets.map((target) => target.id));
  assert(targetIds.size === config.targets.length, "protocol/codegen.json contains duplicate generator targets");
  for (const required of ["rust", "typescript", "dart", "python"]) {
    assert(targetIds.has(required), `protocol/codegen.json is missing ${required} generator target`);
  }
  for (const target of config.targets) {
    assert(isObject(target), "protocol generator target must be an object");
    assert(target.interface === GENERATOR_TARGET_INTERFACE, `${target.id} uses an unsupported generator target interface`);
    assert(["active", "compatibility", "planned"].includes(target.status), `${target.id} has invalid generator target status`);
    const adapter = generatorTargetRegistry[target.id];
    assert(adapter, `protocol/codegen.json declares an unknown generator target: ${target.id}`);
    assert(adapter.interface === target.interface, `${target.id} target registry interface mismatch`);
    if (target.status === "planned") {
      assert(!adapter.implemented, `${target.id} is marked planned but has an implemented adapter`);
    } else {
      assert(adapter.implemented, `${target.id} is marked ${target.status} without an implemented adapter`);
    }
  }
  assert(Array.isArray(config.packages) && config.packages.length > 0, "protocol/codegen.json packages must be non-empty");

  const packageIds = new Set();
  const records = [];
  const schemasByPath = new Map();
  for (const packageConfig of config.packages) {
    assert(isObject(packageConfig), "protocol package entry must be an object");
    assert(!packageIds.has(packageConfig.id), `duplicate protocol package ${packageConfig.id}`);
    packageIds.add(packageConfig.id);
    assert(["core", "agent", "pet", "provider", "gateway", "channel", "desktop"].includes(packageConfig.layer), `${packageConfig.id} has invalid layer`);
    assert(Number.isInteger(packageConfig.version) && packageConfig.version >= 0, `${packageConfig.id} has invalid version`);
    assert(Array.isArray(packageConfig.dependencies), `${packageConfig.id} dependencies must be an array`);
    const publicTypes = packageConfig.publicTypes ?? [];
    assert(Array.isArray(publicTypes), `${packageConfig.id} publicTypes must be an array`);
    assert(new Set(publicTypes).size === publicTypes.length, `${packageConfig.id} publicTypes contains duplicates`);
    for (const name of publicTypes) {
      assert(typeof name === "string" && /^[A-Z][A-Za-z0-9]*$/.test(name), `${packageConfig.id} publicTypes contains invalid type ${name}`);
    }
    assert(isObject(packageConfig.outputs) && Object.keys(packageConfig.outputs).length > 0, `${packageConfig.id} outputs must be non-empty`);
    for (const target of Object.keys(packageConfig.outputs)) {
      assert(targetIds.has(target), `${packageConfig.id} uses undeclared generator target ${target}`);
    }
    const schemaPath = resolve(protocolRoot, packageConfig.schema);
    const manifestPath = resolve(protocolRoot, packageConfig.manifest);
    const record = {
      packageConfig,
      schemaPath,
      manifestPath,
      schema: await readJson(schemaPath),
      manifest: await readJson(manifestPath),
    };
    for (const name of publicTypes) {
      assert(record.schema.$defs?.[name], `${packageConfig.id} publicTypes references unknown definition ${name}`);
    }
    records.push(record);
    schemasByPath.set(schemaPath, record);
  }
  for (const record of records) {
    for (const dependency of record.packageConfig.dependencies) {
      assert(packageIds.has(dependency), `${record.packageConfig.id} references unknown dependency ${dependency}`);
    }
  }
  const model = { config, records, schemasByPath, recordsById: new Map(records.map((record) => [record.packageConfig.id, record])) };
  for (const record of records) validateSchema(record, model);
  for (const record of records) validateManifest(record, model);
  for (const record of records) validatePackageReferences(record, model);
  for (const record of records) await validateFixtures(record, model);
  model.protocolIr = buildProtocolIr(model);
  return model;
}

function words(value) {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1 $2").split(/[^A-Za-z0-9]+/).filter(Boolean);
}

function pascalCase(value) {
  return words(value).map((word) => word[0].toUpperCase() + word.slice(1)).join("");
}

function camelCase(value) {
  const pascal = pascalCase(value);
  return pascal[0].toLowerCase() + pascal.slice(1);
}

function snakeCase(value) {
  return words(value).map((word) => word.toLowerCase()).join("_");
}

function definitionName(referenceObject, sourcePath, model) {
  return referenceTarget(referenceObject, sourcePath, model, "codegen reference").name;
}

function rustType(node, record, model, definition) {
  if (node.$ref) return parseReference(node.$ref, record.schemaPath, model, "Rust type").name;
  const nullableReference = nullableReferenceVariant(node);
  if (nullableReference) return `Option<${rustType(nullableReference, record, model)}>`;
  if (node.type === "string") return "String";
  if (node.type === "boolean") return "bool";
  if (node.type === "integer") {
    if (definition === "ProtocolVersion") return "u32";
    return node.minimum !== undefined && node.minimum >= 0 ? "u64" : "i64";
  }
  if (node.type === "array") return `Vec<${rustType(node.items, record, model)}>`;
  if (node.type === "null") return "()";
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) {
    return "BTreeMap<String, serde_json::Value>";
  }
  if (node.type === "object" && node.properties === undefined && isObject(node.additionalProperties)) {
    return `BTreeMap<String, ${rustType(node.additionalProperties, record, model)}>`;
  }
  fail(`cannot generate Rust type for ${record.packageConfig.id}: ${JSON.stringify(node)}`);
}

function typeScriptType(node, record, model) {
  if (node.$ref) return parseReference(node.$ref, record.schemaPath, model, "TypeScript type").name;
  const nullableReference = nullableReferenceVariant(node);
  if (nullableReference) return `${typeScriptType(nullableReference, record, model)} | null`;
  if (node.type === "string") return "string";
  if (node.type === "boolean") return "boolean";
  if (node.type === "integer") return "number";
  if (node.type === "array") return `Array<${typeScriptType(node.items, record, model)}>`;
  if (node.type === "null") return "null";
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) return "Record<string, JsonValue>";
  if (node.type === "object" && node.properties === undefined && isObject(node.additionalProperties)) {
    return `Record<string, ${typeScriptType(node.additionalProperties, record, model)}>`;
  }
  fail(`cannot generate TypeScript type for ${record.packageConfig.id}: ${JSON.stringify(node)}`);
}

function reachableDefinitionSets(model) {
  const reachable = new Map(model.records.map((record) => [record.packageConfig.id, new Set()]));
  const pending = [];
  const enqueue = (target) => {
    const names = reachable.get(target.record.packageConfig.id);
    if (names.has(target.name)) return;
    names.add(target.name);
    pending.push(target);
  };
  const enqueueReference = (reference, sourcePath, location) => {
    enqueue(parseReference(reference, sourcePath, model, location));
  };

  for (const record of model.records) {
    visitRefs(record.manifest, (reference) => {
      enqueueReference(reference, record.manifestPath, `${record.packageConfig.id} manifest public root`);
    });
    for (const name of record.packageConfig.publicTypes ?? []) {
      enqueue({ name, node: record.schema.$defs[name], record });
    }
  }

  while (pending.length > 0) {
    const target = pending.pop();
    visitRefs(target.node, (reference) => {
      enqueueReference(
        reference,
        target.record.schemaPath,
        `${target.record.packageConfig.id}.${target.name} dependency`,
      );
    });
  }
  return reachable;
}

function reachableDefinitionNames(record, model) {
  return [...reachableDefinitionSets(model).get(record.packageConfig.id)].sort();
}

function externalReferences(record, model) {
  const byPackage = new Map();
  const add = (reference, sourcePath) => {
    const target = parseReference(reference, sourcePath, model, `${record.packageConfig.id} external import`);
    if (target.record.packageConfig.id === record.packageConfig.id) return;
    if (!byPackage.has(target.record.packageConfig.id)) byPackage.set(target.record.packageConfig.id, new Set());
    byPackage.get(target.record.packageConfig.id).add(target.name);
  };
  visitRefs(record.manifest, (reference) => add(reference, record.manifestPath));
  for (const name of reachableDefinitionNames(record, model)) {
    visitRefs(record.schema.$defs[name], (reference) => add(reference, record.schemaPath));
  }
  return byPackage;
}

function normalizeConstraints(node) {
  return Object.fromEntries(
    ["minimum", "maximum", "minLength", "maxLength", "minItems", "pattern", "uniqueItems"]
      .filter((keyword) => node[keyword] !== undefined)
      .map((keyword) => [keyword, node[keyword]]),
  );
}

function normalizeType(node, record, model, location) {
  if (node.$ref) {
    const target = parseReference(node.$ref, record.schemaPath, model, location);
    return {
      kind: "named",
      packageId: target.record.packageConfig.id,
      name: target.name,
    };
  }
  const nullableReference = nullableReferenceVariant(node);
  if (nullableReference) {
    return {
      kind: "nullable",
      value: normalizeType(nullableReference, record, model, `${location}.nullable`),
    };
  }
  if (node.type === "array") {
    return {
      kind: "list",
      items: normalizeType(node.items, record, model, `${location}.items`),
      constraints: normalizeConstraints(node),
    };
  }
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) {
    return { kind: "jsonObject" };
  }
  if (node.type === "object" && node.properties === undefined && isObject(node.additionalProperties)) {
    return {
      kind: "map",
      values: normalizeType(node.additionalProperties, record, model, `${location}.additionalProperties`),
    };
  }
  if (["string", "integer", "boolean", "null"].includes(node.type)) {
    return {
      kind: node.type,
      constraints: normalizeConstraints(node),
    };
  }
  fail(`${location} cannot be normalized as a protocol IR type`);
}

function singletonEnumValue(type, model) {
  if (type.kind !== "named") return undefined;
  const definition = model.recordsById.get(type.packageId)?.schema.$defs[type.name];
  return definition?.type === "string" && definition.enum?.length === 1
    ? definition.enum[0]
    : undefined;
}

function unionDiscriminator(variants, model) {
  const objects = variants.map((variant) => {
    const record = model.recordsById.get(variant.packageId);
    const definition = record?.schema.$defs[variant.name];
    return definition?.type === "object" ? { record, definition } : undefined;
  });
  if (objects.some((value) => value === undefined)) return undefined;
  const commonRequired = objects[0].definition.required ?? [];
  for (const field of commonRequired) {
    const values = objects.map(({ record, definition }) => {
      if (!(definition.required ?? []).includes(field)) return undefined;
      const property = definition.properties?.[field];
      if (!property) return undefined;
      return singletonEnumValue(
        normalizeType(property, record, model, `union discriminator ${field}`),
        model,
      );
    });
    if (values.every((value) => value !== undefined) && new Set(values).size === values.length) {
      return {
        field,
        variants: variants.map((variant, index) => ({ ...variant, value: values[index] })),
      };
    }
  }
  return undefined;
}

function buildDefinitionIr(name, node, record, model) {
  const base = {
    name,
    packageId: record.packageConfig.id,
    description: node.description,
  };
  if (nullableReferenceVariant(node)) {
    return { ...base, kind: "alias", type: normalizeType(node, record, model, `${record.packageConfig.id}.${name}`) };
  }
  if (node.oneOf) {
    const variants = node.oneOf.map((variant, index) => {
      const target = referenceTarget(
        variant,
        record.schemaPath,
        model,
        `${record.packageConfig.id}.${name}.oneOf[${index}]`,
      );
      return { kind: "named", packageId: target.record.packageConfig.id, name: target.name };
    });
    const discriminator = unionDiscriminator(variants, model);
    assert(
      discriminator,
      `${record.packageConfig.id}.${name} is an untagged or ambiguous oneOf; protocol unions require one common required singleton-enum discriminator`,
    );
    for (const variant of variants) {
      const definition = model.recordsById.get(variant.packageId)?.schema.$defs[variant.name];
      assert(
        definition?.type === "object" && definition.additionalProperties === false,
        `${record.packageConfig.id}.${name} variant ${variant.name} must be a closed object`,
      );
    }
    return { ...base, kind: "union", variants, discriminator };
  }
  if (node.enum) return { ...base, kind: "enum", values: [...node.enum], constraints: normalizeConstraints(node) };
  if (node.type === "object" && node.properties !== undefined) {
    const required = new Set(node.required ?? []);
    return {
      ...base,
      kind: "object",
      closed: node.additionalProperties === false,
      fields: Object.entries(node.properties).map(([wireName, field]) => ({
        wireName,
        required: required.has(wireName),
        sensitive: field["x-codepet-sensitive"] === true,
        type: normalizeType(field, record, model, `${record.packageConfig.id}.${name}.${wireName}`),
      })),
    };
  }
  return { ...base, kind: "alias", type: normalizeType(node, record, model, `${record.packageConfig.id}.${name}`) };
}

function namedManifestType(value, sourcePath, model, location) {
  const target = referenceTarget(value, sourcePath, model, location);
  return { kind: "named", packageId: target.record.packageConfig.id, name: target.name };
}

export function buildProtocolIr(model) {
  const reachable = reachableDefinitionSets(model);
  const packages = model.records.map((record) => {
    const definitions = [...reachable.get(record.packageConfig.id)]
      .sort()
      .map((name) => buildDefinitionIr(name, record.schema.$defs[name], record, model));
    const service = record.manifest.kind === "service" ? {
      kind: record.manifest.transport.kind,
      version: record.manifest.version,
      versionField: record.manifest.transport.versionField,
      requestDiscriminator: record.manifest.transport.requestDiscriminator,
      responseDiscriminator: record.manifest.transport.responseDiscriminator,
      eventDiscriminator: record.manifest.transport.eventDiscriminator,
      jsonRpcVersion: record.manifest.transport.jsonRpcVersion,
      eventCursorField: record.manifest.transport.eventCursorField,
      traceContextField: record.manifest.transport.traceContext?.field,
      traceContextType: record.manifest.transport.traceContext
        ? namedManifestType(record.manifest.transport.traceContext.type, record.manifestPath, model, `${record.packageConfig.id}.traceContext`)
        : undefined,
      protocolVersionType: record.manifest.transport.protocolVersion
        ? namedManifestType(record.manifest.transport.protocolVersion, record.manifestPath, model, `${record.packageConfig.id}.protocolVersion`)
        : undefined,
      requestIdType: namedManifestType(record.manifest.transport.requestId, record.manifestPath, model, `${record.packageConfig.id}.requestId`),
      eventCursorType: record.manifest.transport.eventCursor
        ? namedManifestType(record.manifest.transport.eventCursor, record.manifestPath, model, `${record.packageConfig.id}.eventCursor`)
        : undefined,
      errorType: namedManifestType(record.manifest.error, record.manifestPath, model, `${record.packageConfig.id}.error`),
      capabilityType: record.manifest.capabilities
        ? namedManifestType(record.manifest.capabilities.type, record.manifestPath, model, `${record.packageConfig.id}.capabilityType`)
        : undefined,
      methods: record.manifest.methods.map((method) => ({
        name: method.name,
        direction: method.direction,
        idempotency: method.idempotency,
        dispatchLane: method.dispatchLane ?? "normal",
        capability: method.capability,
        requestType: namedManifestType(method.request, record.manifestPath, model, `${record.packageConfig.id}.${method.name}.request`),
        responseType: namedManifestType(method.response, record.manifestPath, model, `${record.packageConfig.id}.${method.name}.response`),
      })),
      events: record.manifest.events.map((event) => ({
        name: event.name,
        direction: event.direction,
        delivery: event.delivery,
        scope: event.scope,
        payloadType: namedManifestType(event.payload, record.manifestPath, model, `${record.packageConfig.id}.${event.name}.payload`),
      })),
    } : undefined;
    return {
      id: record.packageConfig.id,
      layer: record.packageConfig.layer,
      version: record.packageConfig.version,
      dependencies: [...record.packageConfig.dependencies],
      definitions,
      service,
    };
  });
  return {
    interface: GENERATOR_TARGET_INTERFACE,
    packages,
    packagesById: new Map(packages.map((value) => [value.id, value])),
  };
}

function rustDependencyCrate(targetRecord) {
  const output = targetRecord.packageConfig.outputs.rust;
  assert(output, `${targetRecord.packageConfig.id} has no Rust output for dependency import`);
  const segments = output.split("/");
  const srcIndex = segments.lastIndexOf("src");
  assert(srcIndex > 0, `${targetRecord.packageConfig.id} Rust output must be inside src`);
  return segments[srcIndex - 1].replaceAll("-", "_");
}

function rustImports(record, model) {
  const imports = [];
  for (const [packageId, names] of [...externalReferences(record, model)].sort(([left], [right]) => left.localeCompare(right))) {
    const target = model.recordsById.get(packageId);
    const crate = rustDependencyCrate(target);
    if (target.packageConfig.layer === "agent") {
      imports.push(`pub use ${crate}::*;`);
    } else {
      imports.push(`pub use ${crate}::{${[...names].sort().join(", ")}};`);
    }
  }
  return imports.join("\n");
}

function rustRedactedDebug(name, node) {
  const fields = Object.entries(node.properties).map(([field, fieldSchema]) => {
    const value = fieldSchema["x-codepet-sensitive"] === true
      ? `&"<redacted>"`
      : `&self.${snakeCase(field)}`;
    return `            .field("${snakeCase(field)}", ${value})`;
  }).join("\n");
  return `impl std::fmt::Debug for ${name} {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("${name}")
${fields}
            .finish()
    }
}`;
}

function rustDefinitions(record, model) {
  const blocks = [];
  for (const name of reachableDefinitionNames(record, model)) {
    const node = record.schema.$defs[name];
    if (node.oneOf) {
      const variants = node.oneOf.map((variant) => {
        const target = parseReference(variant.$ref, record.schemaPath, model, `Rust oneOf ${name}`);
        return `    ${target.name}(${target.name}),`;
      }).join("\n");
      blocks.push(`#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(untagged)]\npub enum ${name} {\n${variants}\n}`);
    } else if (node.enum) {
      const variants = node.enum.map((value) => `    #[serde(rename = "${value}")]\n    ${pascalCase(value)},`).join("\n");
      blocks.push(`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]\npub enum ${name} {\n${variants}\n}`);
    } else if (node.type === "object" && node.properties !== undefined) {
      const required = new Set(node.required ?? []);
      const hasSensitiveFields = Object.values(node.properties).some(
        (property) => property["x-codepet-sensitive"] === true,
      );
      const fields = Object.entries(node.properties).map(([field, fieldSchema]) => {
        const baseType = rustType(fieldSchema, record, model);
        const optional = !required.has(field);
        const attribute = optional ? "    #[serde(skip_serializing_if = \"Option::is_none\")]\n" : "";
        const wireName = field !== camelCase(field) ? `    #[serde(rename = "${field}")]\n` : "";
        return `${attribute}${wireName}    pub ${snakeCase(field)}: ${optional ? `Option<${baseType}>` : baseType},`;
      }).join("\n");
      const denyUnknown = node.additionalProperties === false ? "\n#[serde(deny_unknown_fields)]" : "";
      const debugDerive = hasSensitiveFields ? "" : "Debug, ";
      const debugImplementation = hasSensitiveFields ? `\n\n${rustRedactedDebug(name, node)}` : "";
      blocks.push(`#[derive(Clone, ${debugDerive}PartialEq, Serialize, Deserialize)]\n#[serde(rename_all = "camelCase")]${denyUnknown}\npub struct ${name} {\n${fields}\n}${debugImplementation}`);
    } else {
      blocks.push(`pub type ${name} = ${rustType(node, record, model, name)};`);
    }
  }
  return blocks.join("\n\n");
}

function protocolEnums(record, model) {
  const methodVariants = record.manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]\n    ${pascalCase(method.name)},`).join("\n");
  const methodAsStr = record.manifest.methods.map((method) => `            Self::${pascalCase(method.name)} => "${method.name}",`).join("\n");
  const methodFromStr = record.manifest.methods.map((method) => `            "${method.name}" => Ok(Self::${pascalCase(method.name)}),`).join("\n");
  const methodDispatchLanes = record.manifest.methods.map((method) => {
    const lane = method.dispatchLane === "control" ? "Control" : "Normal";
    return `            Self::${pascalCase(method.name)} => ProtocolDispatchLane::${lane},`;
  }).join("\n");
  const eventVariants = record.manifest.events.map((event) => `    #[serde(rename = "${event.name}")]\n    ${pascalCase(event.name)},`).join("\n");
  const eventAsStr = record.manifest.events.map((event) => `            Self::${pascalCase(event.name)} => "${event.name}",`).join("\n");
  const eventFromStr = record.manifest.events.map((event) => `            "${event.name}" => Ok(Self::${pascalCase(event.name)}),`).join("\n");
  const dispatchLaneType = record.manifest.transport.kind === "json-rpc-2.0"
    ? `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolDispatchLane {
    Normal,
    Control,
}

`
    : "";
  const dispatchLaneMethod = record.manifest.transport.kind === "json-rpc-2.0"
    ? `

    pub const fn dispatch_lane(self) -> ProtocolDispatchLane {
        match self {
${methodDispatchLanes}
        }
    }`
    : "";
  let capabilityMethod = "";
  if (record.manifest.capabilities) {
    const capabilityType = referenceTarget(record.manifest.capabilities.type, record.manifestPath, model, "capability type").name;
    const capabilityArms = record.manifest.methods.map((method) => {
      const value = method.capability ? `Some(${capabilityType}::${pascalCase(method.capability)})` : "None";
      return `            Self::${pascalCase(method.name)} => ${value},`;
    }).join("\n");
    capabilityMethod = `

    pub const fn capability(self) -> Option<${capabilityType}> {
        match self {
${capabilityArms}
        }
    }`;
  }
  return `${dispatchLaneType}#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolMethod {
${methodVariants}
}

impl ProtocolMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
${methodAsStr}
        }
    }${dispatchLaneMethod}${capabilityMethod}
}

impl std::str::FromStr for ProtocolMethod {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
${methodFromStr}
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolEventName {
${eventVariants}
}

impl ProtocolEventName {
    pub const fn as_str(self) -> &'static str {
        match self {
${eventAsStr}
        }
    }
}

impl std::str::FromStr for ProtocolEventName {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
${eventFromStr}
            _ => Err(()),
        }
    }
}`;
}

function serverTrait(record, model) {
  const methods = record.manifest.methods.map((method) => {
    const request = definitionName(method.request, record.manifestPath, model);
    const response = definitionName(method.response, record.manifestPath, model);
    return `    fn ${snakeCase(method.name)}<'a>(&'a self, _request: ${request}) -> ProtocolFuture<'a, ${response}> {\n        Box::pin(async { Err(method_not_implemented("${method.name}")) })\n    }`;
  }).join("\n\n");
  return `pub trait ProtocolServer: Send + Sync {
${methods}
}

fn method_not_implemented(method: &str) -> ProtocolError {
    ProtocolError {
        code: "method_not_implemented".to_string(),
        message: format!("protocol method is not implemented: {method}"),
        retryable: false,
        details: None,
    }
}`;
}

function clientSupport(record, model) {
  const methods = record.manifest.methods.map((method) => {
    const request = definitionName(method.request, record.manifestPath, model);
    const response = definitionName(method.response, record.manifestPath, model);
    return `    pub fn ${snakeCase(method.name)}<'a>(&'a self, request: ${request}) -> ProtocolFuture<'a, ${response}> {
        Box::pin(async move {
            let params = serde_json::to_value(request).map_err(|error| codec_error("encode request params", error))?;
            let result = self.transport.request(ProtocolMethod::${pascalCase(method.name)}, params).await?;
            serde_json::from_value(result).map_err(|error| codec_error("decode response result", error))
        })
    }`;
  }).join("\n\n");
  const inboundTransport = record.manifest.transport.kind === "json-rpc-2.0"
    ? `

pub type ProtocolInboundFuture<'a> = Pin<Box<dyn Future<Output = Result<ProviderWireMessage, JsonRpcInboundError>> + Send + 'a>>;`
    : "";
  const inboundMethod = record.manifest.transport.kind === "json-rpc-2.0"
    ? "\n    fn next_message<'a>(&'a self) -> ProtocolInboundFuture<'a>;"
    : "";
  const inboundClient = record.manifest.transport.kind === "json-rpc-2.0" ? `

    pub fn next_message<'a>(&'a self) -> ProtocolInboundFuture<'a> {
        self.transport.next_message()
    }` : "";
  return `pub type ProtocolTransportFuture<'a> = Pin<Box<dyn Future<Output = Result<serde_json::Value, ProtocolError>> + Send + 'a>>;${inboundTransport}

pub trait ProtocolTransport: Send + Sync {
    fn request<'a>(&'a self, method: ProtocolMethod, params: serde_json::Value) -> ProtocolTransportFuture<'a>;${inboundMethod}
}

pub struct ProtocolClient<T> {
    transport: T,
}

impl<T> ProtocolClient<T> {
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub const fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T: ProtocolTransport> ProtocolClient<T> {
${methods}${inboundClient}
}`;
}

function generateCodepetEnvelope(record, model, role = "both") {
  const { manifest, manifestPath } = record;
  const requestVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]
    ${pascalCase(method.name)} {
        #[serde(rename = "${manifest.transport.versionField}")]
        protocol_version: ProtocolVersion,
        id: RequestId,
        params: ${definitionName(method.request, manifestPath, model)},
    },`).join("\n");
  const responseVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]
    ${pascalCase(method.name)} {
        #[serde(rename = "${manifest.transport.versionField}")]
        protocol_version: ProtocolVersion,
        id: RequestId,
        response: ResponsePayload<${definitionName(method.response, manifestPath, model)}>,
    },`).join("\n");
  const cursorField = manifest.transport.eventCursorField;
  const cursorType = cursorField ? referenceTarget(manifest.transport.eventCursor, manifestPath, model, "event cursor").name : undefined;
  const eventVariants = manifest.events.map((event) => {
    const cursor = cursorField ? `\n        #[serde(rename = "${cursorField}")]\n        ${snakeCase(cursorField)}: ${cursorType},` : "";
    return `    #[serde(rename = "${event.name}")]
    ${pascalCase(event.name)} {
        #[serde(rename = "${manifest.transport.versionField}")]
        protocol_version: ProtocolVersion,${cursor}
        payload: ${definitionName(event.payload, manifestPath, model)},
    },`;
  }).join("\n");
  const dispatchArms = manifest.methods.map((method) => {
    const variant = pascalCase(method.name);
    return `        ProtocolRequest::${variant} { protocol_version, id, params } => {
            let response = match server.${snakeCase(method.name)}(params).await {
                Ok(result) => ResponsePayload::Ok { result },
                Err(error) => ResponsePayload::Error { error },
            };
            ProtocolResponse::${variant} { protocol_version, id, response }
        }`;
  }).join(",\n");
  const serverBlock = role === "client" ? "" : `${serverTrait(record, model)}

pub async fn dispatch<S: ProtocolServer + ?Sized>(server: &S, request: ProtocolRequest) -> ProtocolResponse {
    match request {
${dispatchArms}
    }
}

pub struct ProtocolDispatcher<S> {
    server: S,
}

impl<S> ProtocolDispatcher<S> {
    pub const fn new(server: S) -> Self {
        Self { server }
    }

    pub const fn server(&self) -> &S {
        &self.server
    }
}

impl<S: ProtocolServer> ProtocolDispatcher<S> {
    pub async fn dispatch(&self, request: ProtocolRequest) -> ProtocolResponse {
        dispatch(&self.server, request).await
    }
}`;
  const clientBlock = role === "server" ? "" : clientSupport(record, model);
  return `${protocolEnums(record, model)}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ResponsePayload<T> {
    Ok { result: T },
    Error { error: ProtocolError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.requestDiscriminator}")]
pub enum ProtocolRequest {
${requestVariants}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.responseDiscriminator}")]
pub enum ProtocolResponse {
${responseVariants}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.eventDiscriminator}")]
pub enum ProtocolEvent {
${eventVariants}
}

pub type ProtocolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProtocolError>> + Send + 'a>>;

${serverBlock}

${clientBlock}

pub fn encode_request(value: &ProtocolRequest) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(value).map_err(|error| codec_error("encode request", error))
}

pub fn decode_request(value: &[u8]) -> Result<ProtocolRequest, ProtocolError> {
    serde_json::from_slice(value).map_err(|error| codec_error("decode request", error))
}

pub fn encode_response(value: &ProtocolResponse) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(value).map_err(|error| codec_error("encode response", error))
}

pub fn decode_response(value: &[u8]) -> Result<ProtocolResponse, ProtocolError> {
    serde_json::from_slice(value).map_err(|error| codec_error("decode response", error))
}

pub fn encode_event(value: &ProtocolEvent) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(value).map_err(|error| codec_error("encode event", error))
}

pub fn decode_event(value: &[u8]) -> Result<ProtocolEvent, ProtocolError> {
    serde_json::from_slice(value).map_err(|error| codec_error("decode event", error))
}

fn codec_error(context: &str, error: serde_json::Error) -> ProtocolError {
    ProtocolError {
        code: "protocol_codec_error".to_string(),
        message: format!("{context}: {error}"),
        retryable: false,
        details: None,
    }
}`;
}

function generateJsonRpc(record, model, role = "both") {
  const { manifest, manifestPath } = record;
  const cursorField = manifest.transport.eventCursorField;
  const cursorType = cursorField ? referenceTarget(manifest.transport.eventCursor, manifestPath, model, "JSON-RPC event cursor").name : undefined;
  const traceConfig = manifest.transport.traceContext;
  const traceField = traceConfig?.field;
  const traceType = traceConfig ? referenceTarget(traceConfig.type, manifestPath, model, "JSON-RPC trace context").name : undefined;
  const traceAllowed = traceField ? `, "${traceField}"` : "";
  const requestVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]
    ${pascalCase(method.name)} {
        jsonrpc: String,
        id: RequestId,
        params: ${definitionName(method.request, manifestPath, model)},
    },`).join("\n");
  const eventVariants = manifest.events.map((event) => `    #[serde(rename = "${event.name}")]
    ${pascalCase(event.name)} {
        jsonrpc: String,
        params: ${cursorField ? `ProtocolEventParams<${definitionName(event.payload, manifestPath, model)}>` : definitionName(event.payload, manifestPath, model)},
    },`).join("\n");
  const requestVersions = manifest.methods.map((method) => `            Self::${pascalCase(method.name)} { jsonrpc, .. } => jsonrpc,`).join("\n");
  const requestIds = manifest.methods.map((method) => `            Self::${pascalCase(method.name)} { id, .. } => id,`).join("\n");
  const requestMethods = manifest.methods.map((method) => `            Self::${pascalCase(method.name)} { .. } => ProtocolMethod::${pascalCase(method.name)},`).join("\n");
  const requestConstructors = manifest.methods.map((method) => `            ProtocolMethod::${pascalCase(method.name)} => Ok(Self::${pascalCase(method.name)} {
                jsonrpc,
                id,
                params: serde_json::from_value(params).map_err(|error| codec_error("decode ${method.name} request params", error))?,
            }),`).join("\n");
  const eventVersions = manifest.events.map((event) => `            Self::${pascalCase(event.name)} { jsonrpc, .. } => jsonrpc,`).join("\n");
  const eventCursorMethods = cursorField ? `

    pub fn event_cursor(&self) -> &${cursorType} {
        match self {
${manifest.events.map((event) => `            Self::${pascalCase(event.name)} { params, .. } => &params.${snakeCase(cursorField)},`).join("\n")}
        }
    }

    pub fn set_event_cursor(&mut self, cursor: ${cursorType}) {
        match self {
${manifest.events.map((event) => `            Self::${pascalCase(event.name)} { jsonrpc, params } => {
                *jsonrpc = "${manifest.transport.jsonRpcVersion}".to_string();
                params.${snakeCase(cursorField)} = cursor;
            },`).join("\n")}
        }
    }` : "";
  const dispatchArms = manifest.methods.map((method) => {
    const variant = pascalCase(method.name);
    return `        ProtocolRequest::${variant} { jsonrpc, id, params } => {
            let response = match server.${snakeCase(method.name)}(params).await {
                Ok(result) => match serde_json::to_value(result) {
                    Ok(result) => JsonRpcResponsePayload::Ok { result },
                    Err(error) => JsonRpcResponsePayload::Error { error: rpc_codec_error("encode response result", error) },
                },
                Err(error) => JsonRpcResponsePayload::Error { error: rpc_method_error(error) },
            };
            JsonRpcResponse { jsonrpc, id: Some(id), response }
        }`;
  }).join(",\n");
  const eventParams = cursorField ? `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ProtocolEventParams<T> {
    #[serde(rename = "${cursorField}")]
    pub ${snakeCase(cursorField)}: ${cursorType},
    pub payload: T,
}

` : "";
  const serverBlock = role === "client" ? "" : `${serverTrait(record, model)}

pub async fn dispatch<S: ProtocolServer + ?Sized>(server: &S, request: ProtocolRequest) -> JsonRpcResponse {
    match request {
${dispatchArms}
    }
}

pub struct ProtocolDispatcher<S> {
    server: S,
}

impl<S> ProtocolDispatcher<S> {
    pub const fn new(server: S) -> Self {
        Self { server }
    }

    pub const fn server(&self) -> &S {
        &self.server
    }
}

impl<S: ProtocolServer> ProtocolDispatcher<S> {
    pub async fn dispatch(&self, request: ProtocolRequest) -> JsonRpcResponse {
        dispatch(&self.server, request).await
    }
}`;
  const clientBlock = role === "server" ? "" : clientSupport(record, model);
  return `${protocolEnums(record, model)}

pub const JSON_RPC_PARSE_ERROR: i64 = -32700;
pub const JSON_RPC_INVALID_REQUEST: i64 = -32600;
pub const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
pub const JSON_RPC_INVALID_PARAMS: i64 = -32602;
pub const JSON_RPC_INTERNAL_ERROR: i64 = -32603;
pub const DEFAULT_MAX_JSON_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.requestDiscriminator}")]
pub enum ProtocolRequest {
${requestVariants}
}

impl ProtocolRequest {
    pub fn from_method_params(
        method: ProtocolMethod,
        id: RequestId,
        params: serde_json::Value,
    ) -> Result<Self, ProtocolError> {
        let jsonrpc = "${manifest.transport.jsonRpcVersion}".to_string();
        match method {
${requestConstructors}
        }
    }

    pub fn jsonrpc_version(&self) -> &str {
        match self {
${requestVersions}
        }
    }

    pub fn id(&self) -> &RequestId {
        match self {
${requestIds}
        }
    }

    pub const fn method(&self) -> ProtocolMethod {
        match self {
${requestMethods}
        }
    }
}

${eventParams}#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.eventDiscriminator}")]
pub enum ProtocolEvent {
${eventVariants}
}

impl ProtocolEvent {
    pub fn jsonrpc_version(&self) -> &str {
        match self {
${eventVersions}
        }
    }${eventCursorMethods}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponsePayload {
    Ok { result: serde_json::Value },
    Error { error: RpcError },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub response: JsonRpcResponsePayload,
}

impl<'de> Deserialize<'de> for JsonRpcResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        parse_jsonrpc_response(value).map_err(|error| serde::de::Error::custom(error.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonRpcRequestRejection {
    pub id: RequestId,
    pub method: String,
    pub error: RpcError,
}

impl JsonRpcRequestRejection {
    pub fn into_response(self) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "${manifest.transport.jsonRpcVersion}".to_string(),
            id: Some(self.id),
            response: JsonRpcResponsePayload::Error { error: self.error },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JsonRpcInboundRequest {
    Typed(ProtocolRequest),
    Rejected(JsonRpcRequestRejection),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderWireMessage {
    Request(JsonRpcInboundRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
    Event(ProtocolEvent),
}

${traceType ? `#[derive(Clone, Debug, PartialEq)]
pub struct ObservedWireMessage {
    pub message: ProviderWireMessage,
    pub trace_context: Option<${traceType}>,
}
` : ""}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonRpcInboundError {
    pub id: Option<RequestId>,
    pub error: RpcError,
}

impl JsonRpcInboundError {
    pub fn into_response(self) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "${manifest.transport.jsonRpcVersion}".to_string(),
            id: self.id,
            response: JsonRpcResponsePayload::Error { error: self.error },
        }
    }
}

impl std::fmt::Display for JsonRpcInboundError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "JSON-RPC error {}: {}", self.error.code, self.error.message)
    }
}

impl std::error::Error for JsonRpcInboundError {}

pub type ProtocolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProtocolError>> + Send + 'a>>;

${serverBlock}

${clientBlock}

fn rpc_method_error(error: ProtocolError) -> RpcError {
    let message = error.message.clone();
    let data = serde_json::to_value(error).ok().and_then(|value| match value {
        serde_json::Value::Object(entries) => Some(entries.into_iter().collect()),
        _ => None,
    });
    RpcError { code: -32000, message, data }
}

fn rpc_codec_error(context: &str, error: serde_json::Error) -> RpcError {
    RpcError { code: JSON_RPC_INTERNAL_ERROR, message: format!("{context}: {error}"), data: None }
}

fn codec_error(context: &str, error: serde_json::Error) -> ProtocolError {
    ProtocolError {
        code: "protocol_codec_error".to_string(),
        message: format!("{context}: {error}"),
        retryable: false,
        details: None,
    }
}

fn inbound_error(id: Option<RequestId>, code: i64, message: impl Into<String>) -> JsonRpcInboundError {
    JsonRpcInboundError {
        id,
        error: RpcError { code, message: message.into(), data: None },
    }
}

fn inbound_protocol_error(error: JsonRpcInboundError) -> ProtocolError {
    ProtocolError {
        code: format!("json_rpc_{}", error.error.code),
        message: error.error.message,
        retryable: false,
        details: error.error.data,
    }
}

fn object_has_only(object: &serde_json::Map<String, serde_json::Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

fn object_request_id(object: &serde_json::Map<String, serde_json::Value>) -> Option<RequestId> {
    object.get("id").and_then(serde_json::Value::as_str).map(str::to_string)
}

fn validate_jsonrpc(version: &str) -> Result<(), ProtocolError> {
    if version == "${manifest.transport.jsonRpcVersion}" {
        return Ok(());
    }
    Err(ProtocolError {
        code: "unsupported_jsonrpc_version".to_string(),
        message: format!("unsupported JSON-RPC version: {version}"),
        retryable: false,
        details: None,
    })
}

fn validate_inbound_jsonrpc(object: &serde_json::Map<String, serde_json::Value>, id: Option<RequestId>) -> Result<(), JsonRpcInboundError> {
    match object.get("jsonrpc").and_then(serde_json::Value::as_str) {
        Some("${manifest.transport.jsonRpcVersion}") => Ok(()),
        _ => Err(inbound_error(id, JSON_RPC_INVALID_REQUEST, "jsonrpc must be exactly ${manifest.transport.jsonRpcVersion}")),
    }
}

fn parse_jsonrpc_response(value: serde_json::Value) -> Result<JsonRpcResponse, JsonRpcInboundError> {
    let object = value.as_object().ok_or_else(|| inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON-RPC response must be an object"))?;
    let id = match object.get("id") {
        Some(serde_json::Value::String(value)) => Some(value.clone()),
        Some(serde_json::Value::Null) => None,
        _ => return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON-RPC response id must be a string or null")),
    };
    validate_inbound_jsonrpc(object, id.clone())?;
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    if has_result == has_error {
        return Err(inbound_error(id, JSON_RPC_INVALID_REQUEST, "JSON-RPC response must contain exactly one of result or error"));
    }
    if !object_has_only(object, &["jsonrpc", "id", if has_result { "result" } else { "error" }${traceAllowed}]) {
        return Err(inbound_error(id, JSON_RPC_INVALID_REQUEST, "JSON-RPC response contains unknown fields"));
    }
    let response = if has_result {
        JsonRpcResponsePayload::Ok { result: object["result"].clone() }
    } else {
        let error = serde_json::from_value(object["error"].clone())
            .map_err(|error| inbound_error(id.clone(), JSON_RPC_INVALID_REQUEST, format!("invalid JSON-RPC error object: {error}")))?;
        JsonRpcResponsePayload::Error { error }
    };
    Ok(JsonRpcResponse { jsonrpc: "${manifest.transport.jsonRpcVersion}".to_string(), id, response })
}

pub fn decode_wire_message(value: &[u8]) -> Result<ProviderWireMessage, JsonRpcInboundError> {
    let value: serde_json::Value = serde_json::from_slice(value)
        .map_err(|error| inbound_error(None, JSON_RPC_PARSE_ERROR, format!("parse error: {error}")))?;
    let object = value.as_object().ok_or_else(|| inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON-RPC message must be an object"))?;
    let id = object_request_id(object);
    validate_inbound_jsonrpc(object, id.clone())?;

    if let Some(method_value) = object.get("method") {
        let method = method_value.as_str().ok_or_else(|| inbound_error(id.clone(), JSON_RPC_INVALID_REQUEST, "JSON-RPC method must be a string"))?.to_string();
        if object.contains_key("id") {
            let id = id.ok_or_else(|| inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON-RPC request id must be a string"))?;
            if !object_has_only(object, &["jsonrpc", "id", "method", "params"${traceAllowed}]) {
                return Err(inbound_error(Some(id), JSON_RPC_INVALID_REQUEST, "JSON-RPC request contains unknown fields"));
            }
            if method.parse::<ProtocolMethod>().is_err() {
                return Ok(ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(JsonRpcRequestRejection {
                    id,
                    method: method.clone(),
                    error: RpcError { code: JSON_RPC_METHOD_NOT_FOUND, message: format!("method not found: {method}"), data: None },
                })));
            }
            return match serde_json::from_value(${traceField ? `{ let mut typed = value; if let Some(object) = typed.as_object_mut() { object.remove("${traceField}"); } typed }` : "value"}) {
                Ok(request) => Ok(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request))),
                Err(error) => Ok(ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(JsonRpcRequestRejection {
                    id,
                    method: method.clone(),
                    error: RpcError { code: JSON_RPC_INVALID_PARAMS, message: format!("invalid params for {method}: {error}"), data: None },
                }))),
            };
        }

        if !object_has_only(object, &["jsonrpc", "method", "params"${traceAllowed}]) {
            return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON-RPC notification contains unknown fields"));
        }
        if method.parse::<ProtocolEventName>().is_ok() {
            let event = serde_json::from_value(${traceField ? `{ let mut typed = value; if let Some(object) = typed.as_object_mut() { object.remove("${traceField}"); } typed }` : "value"})
                .map_err(|error| inbound_error(None, JSON_RPC_INVALID_PARAMS, format!("invalid event params for {method}: {error}")))?;
            return Ok(ProviderWireMessage::Event(event));
        }
        return Ok(ProviderWireMessage::Notification(JsonRpcNotification {
            jsonrpc: "${manifest.transport.jsonRpcVersion}".to_string(),
            method,
            params: object.get("params").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }

    parse_jsonrpc_response(value).map(ProviderWireMessage::Response)
}

${traceType ? `pub fn decode_observed_wire_message(value: &[u8]) -> Result<ObservedWireMessage, JsonRpcInboundError> {
    let decoded: serde_json::Value = serde_json::from_slice(value)
        .map_err(|error| inbound_error(None, JSON_RPC_PARSE_ERROR, format!("parse error: {error}")))?;
    let trace_context = decoded
        .as_object()
        .and_then(|object| object.get("${traceField}"))
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|error| inbound_error(
            decoded.as_object().and_then(object_request_id),
            JSON_RPC_INVALID_REQUEST,
            format!("invalid JSON-RPC trace context: {error}"),
        ))?;
    let message = decode_wire_message(value)?;
    Ok(ObservedWireMessage { message, trace_context })
}

fn encode_with_trace<T: Serialize>(
    value: &T,
    trace_context: Option<&${traceType}>,
    context: &str,
) -> Result<Vec<u8>, ProtocolError> {
    let mut encoded = serde_json::to_value(value).map_err(|error| codec_error(context, error))?;
    if let Some(trace_context) = trace_context {
        let object = encoded.as_object_mut().ok_or_else(|| ProtocolError {
            code: "protocol_codec_error".to_string(),
            message: format!("{context}: envelope must be an object"),
            retryable: false,
            details: None,
        })?;
        object.insert(
            "${traceField}".to_string(),
            serde_json::to_value(trace_context).map_err(|error| codec_error(context, error))?,
        );
    }
    serde_json::to_vec(&encoded).map_err(|error| codec_error(context, error))
}
` : ""}

pub fn encode_request(value: &ProtocolRequest) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(value.jsonrpc_version())?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode request", error))
}

pub fn decode_request(value: &[u8]) -> Result<ProtocolRequest, ProtocolError> {
    match decode_wire_message(value).map_err(inbound_protocol_error)? {
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => Ok(request),
        ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(rejection)) => Err(inbound_protocol_error(JsonRpcInboundError {
            id: Some(rejection.id),
            error: rejection.error,
        })),
        _ => Err(ProtocolError {
            code: "unexpected_json_rpc_message".to_string(),
            message: "expected JSON-RPC request".to_string(),
            retryable: false,
            details: None,
        }),
    }
}

pub fn encode_response(value: &JsonRpcResponse) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(&value.jsonrpc)?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode response", error))
}

${traceType ? `pub fn encode_response_with_trace(
    value: &JsonRpcResponse,
    trace_context: Option<&${traceType}>,
) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(&value.jsonrpc)?;
    encode_with_trace(value, trace_context, "encode response")
}
` : ""}

pub fn decode_response(value: &[u8]) -> Result<JsonRpcResponse, ProtocolError> {
    match decode_wire_message(value).map_err(inbound_protocol_error)? {
        ProviderWireMessage::Response(response) => Ok(response),
        _ => Err(ProtocolError {
            code: "unexpected_json_rpc_message".to_string(),
            message: "expected JSON-RPC response".to_string(),
            retryable: false,
            details: None,
        }),
    }
}

pub fn encode_event(value: &ProtocolEvent) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(value.jsonrpc_version())?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode event", error))
}

${traceType ? `pub fn encode_event_with_trace(
    value: &ProtocolEvent,
    trace_context: Option<&${traceType}>,
) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(value.jsonrpc_version())?;
    encode_with_trace(value, trace_context, "encode event")
}
` : ""}

pub fn decode_event(value: &[u8]) -> Result<ProtocolEvent, ProtocolError> {
    match decode_wire_message(value).map_err(inbound_protocol_error)? {
        ProviderWireMessage::Event(event) => Ok(event),
        _ => Err(ProtocolError {
            code: "unexpected_json_rpc_message".to_string(),
            message: "expected JSON-RPC event".to_string(),
            retryable: false,
            details: None,
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonLineCodec {
    max_frame_bytes: usize,
}

impl JsonLineCodec {
    pub fn new(max_frame_bytes: usize) -> Result<Self, ProtocolError> {
        if max_frame_bytes == 0 {
            return Err(ProtocolError {
                code: "invalid_frame_limit".to_string(),
                message: "JSON line frame limit must be greater than zero".to_string(),
                retryable: false,
                details: None,
            });
        }
        Ok(Self { max_frame_bytes })
    }

    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    fn frame_payload(&self, mut payload: Vec<u8>) -> Result<Vec<u8>, ProtocolError> {
        if payload.len() > self.max_frame_bytes {
            return Err(ProtocolError {
                code: "json_line_frame_too_large".to_string(),
                message: format!("JSON line frame exceeds {} bytes", self.max_frame_bytes),
                retryable: false,
                details: None,
            });
        }
        if payload.contains(&b'\\n') || payload.contains(&b'\\r') {
            return Err(ProtocolError {
                code: "invalid_json_line_frame".to_string(),
                message: "JSON line payload contains a physical line break".to_string(),
                retryable: false,
                details: None,
            });
        }
        payload.push(b'\\n');
        Ok(payload)
    }

    pub fn encode_message(&self, message: &ProviderWireMessage) -> Result<Vec<u8>, ProtocolError> {
        let payload = match message {
            ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => encode_request(request)?,
            ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(_)) => {
                return Err(ProtocolError {
                    code: "cannot_encode_rejected_request".to_string(),
                    message: "a rejected inbound request is not a wire request".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            ProviderWireMessage::Response(response) => encode_response(response)?,
            ProviderWireMessage::Notification(notification) => {
                validate_jsonrpc(&notification.jsonrpc)?;
                serde_json::to_vec(notification).map_err(|error| codec_error("encode notification", error))?
            }
            ProviderWireMessage::Event(event) => encode_event(event)?,
        };
        self.frame_payload(payload)
    }

    pub fn decode_line(&self, line: &[u8]) -> Result<ProviderWireMessage, JsonRpcInboundError> {
        let mut payload = line;
        if payload.ends_with(b"\\n") {
            payload = &payload[..payload.len() - 1];
            if payload.ends_with(b"\\r") {
                payload = &payload[..payload.len() - 1];
            }
        }
        if payload.len() > self.max_frame_bytes {
            return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, format!("JSON line frame exceeds {} bytes", self.max_frame_bytes)));
        }
        if payload.contains(&b'\\n') || payload.contains(&b'\\r') {
            return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, "JSON line frame contains multiple physical lines"));
        }
        decode_wire_message(payload)
    }

    pub fn read_message<R: BufRead>(&self, reader: &mut R) -> Result<Option<ProviderWireMessage>, JsonRpcInboundError> {
        let mut frame = Vec::with_capacity(self.max_frame_bytes.min(8192));
        let mut oversized = false;
        loop {
            let (consumed, complete) = {
                let available = reader.fill_buf()
                    .map_err(|error| inbound_error(None, JSON_RPC_INTERNAL_ERROR, format!("read JSON line frame: {error}")))?;
                if available.is_empty() {
                    if oversized {
                        return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, format!("JSON line frame exceeds {} bytes", self.max_frame_bytes)));
                    }
                    if frame.is_empty() {
                        return Ok(None);
                    }
                    return self.decode_line(&frame).map(Some);
                }
                let newline = available.iter().position(|byte| *byte == b'\\n');
                let payload_bytes = newline.unwrap_or(available.len());
                if !oversized && frame.len().saturating_add(payload_bytes) > self.max_frame_bytes {
                    oversized = true;
                }
                if !oversized {
                    frame.extend_from_slice(&available[..payload_bytes]);
                }
                (newline.map_or(payload_bytes, |index| index + 1), newline.is_some())
            };
            reader.consume(consumed);
            if complete {
                if oversized {
                    return Err(inbound_error(None, JSON_RPC_INVALID_REQUEST, format!("JSON line frame exceeds {} bytes", self.max_frame_bytes)));
                }
                if frame.ends_with(b"\\r") {
                    frame.pop();
                }
                return self.decode_line(&frame).map(Some);
            }
        }
    }

    pub fn write_message<W: Write>(&self, writer: &mut W, message: &ProviderWireMessage) -> Result<(), ProtocolError> {
        let frame = self.encode_message(message)?;
        writer.write_all(&frame).map_err(|error| ProtocolError {
            code: "json_line_write_failed".to_string(),
            message: format!("write JSON line frame: {error}"),
            retryable: true,
            details: None,
        })
    }
}

impl Default for JsonLineCodec {
    fn default() -> Self {
        Self { max_frame_bytes: DEFAULT_MAX_JSON_LINE_BYTES }
    }
}`;
}

function providerInstanceKindSupport(record) {
  if (record.packageConfig.layer !== "provider") return "";
  return `impl ProviderPluginDescriptor {
    pub fn validate_instance_kinds(&self) -> Result<(), ProtocolError> {
        if self.instance_kinds.is_empty() {
            return Err(ProtocolError {
                code: "invalid_provider_descriptor".to_string(),
                message: "provider descriptor must declare at least one instance kind".to_string(),
                retryable: false,
                details: None,
            });
        }
        for (index, instance_kind) in self.instance_kinds.iter().enumerate() {
            if instance_kind.is_empty() {
                return Err(ProtocolError {
                    code: "invalid_provider_descriptor".to_string(),
                    message: "provider descriptor instance kinds must be non-empty".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            if self.instance_kinds[..index].contains(instance_kind) {
                return Err(ProtocolError {
                    code: "invalid_provider_descriptor".to_string(),
                    message: format!("provider descriptor contains duplicate instance kind: {instance_kind}"),
                    retryable: false,
                    details: None,
                });
            }
        }
        Ok(())
    }

    pub fn supports_instance_kind(&self, instance_kind: &str) -> bool {
        self.instance_kinds.iter().any(|supported| supported == instance_kind)
    }

    pub fn validate_instance_kind(&self, instance_kind: &str) -> Result<(), ProtocolError> {
        self.validate_instance_kinds()?;
        if self.supports_instance_kind(instance_kind) {
            return Ok(());
        }
        Err(ProtocolError {
            code: "unsupported_instance_kind".to_string(),
            message: format!("provider does not support instance kind: {instance_kind}"),
            retryable: false,
            details: None,
        })
    }
}`;
}

function generateRust(record, model, role = "both") {
  const sourceFiles = [record.manifestPath, record.schemaPath].map((file) => relative(repositoryRoot, file).replaceAll("\\", "/")).join(" and ");
  const imports = rustImports(record, model);
  const definitions = rustDefinitions(record, model);
  const collectionsImport = definitions.includes("BTreeMap<") ? "use std::collections::BTreeMap;\n" : "";
  const ioImport = record.manifest.transport?.kind === "json-rpc-2.0" ? "use std::io::{BufRead, Write};\n" : "";
  if (record.manifest.kind === "types") {
    const schemaVersionConstant = `${record.packageConfig.id.replace(/-v\d+$/, "").replaceAll("-", "_").toUpperCase()}_SCHEMA_VERSION`;
    return `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.
// DO NOT EDIT MANUALLY.

use serde::{Deserialize, Serialize};
${collectionsImport}

${imports ? `${imports}\n\n` : ""}pub const ${schemaVersionConstant}: u64 = ${record.manifest.version};

${definitions}
`;
  }
  const service = record.manifest.transport.kind === "json-rpc-2.0"
    ? generateJsonRpc(record, model, role)
    : generateCodepetEnvelope(record, model, role);
  const instanceKindSupport = providerInstanceKindSupport(record);
  const instanceKindBlock = instanceKindSupport ? `${instanceKindSupport}\n\n` : "";
  return `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.
// DO NOT EDIT MANUALLY.

use serde::{Deserialize, Serialize};
${collectionsImport}${ioImport}use std::future::Future;
use std::pin::Pin;

${imports ? `${imports}\n\n` : ""}pub const PROTOCOL_VERSION: ProtocolVersion = ${record.manifest.version};

${definitions}

${instanceKindBlock}${service}
`;
}

function typeScriptDefinitions(record, model) {
  const blocks = ["export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };"];
  for (const name of reachableDefinitionNames(record, model)) {
    const node = record.schema.$defs[name];
    if (node.oneOf) {
      blocks.push(`export type ${name} = ${node.oneOf.map((variant) => parseReference(variant.$ref, record.schemaPath, model, `TypeScript oneOf ${name}`).name).join(" | ")};`);
    } else if (node.enum) {
      blocks.push(`export type ${name} = ${node.enum.map((value) => JSON.stringify(value)).join(" | ")};`);
    } else if (node.type === "object" && node.properties !== undefined) {
      const required = new Set(node.required ?? []);
      const fields = Object.entries(node.properties).map(([field, fieldSchema]) => `  ${field}${required.has(field) ? "" : "?"}: ${typeScriptType(fieldSchema, record, model)};`).join("\n");
      blocks.push(`export interface ${name} {\n${fields}\n}`);
    } else {
      blocks.push(`export type ${name} = ${typeScriptType(node, record, model)};`);
    }
  }
  return blocks.join("\n\n");
}

function typescriptImports(record, model) {
  const outputPath = resolve(outputRoot, record.packageConfig.outputs.typescript);
  const lines = [];
  for (const [packageId, names] of [...externalReferences(record, model)].sort(([left], [right]) => left.localeCompare(right))) {
    const target = model.recordsById.get(packageId);
    const targetOutput = target.packageConfig.outputs.typescript;
    assert(targetOutput, `${record.packageConfig.id} needs TypeScript output from ${packageId}`);
    let importPath = relative(dirname(outputPath), resolve(outputRoot, targetOutput)).replaceAll("\\", "/").replace(/\.ts$/, "");
    if (!importPath.startsWith(".")) importPath = `./${importPath}`;
    const list = [...names].sort().join(", ");
    lines.push(`import type { ${list} } from ${JSON.stringify(importPath)};`);
    if (target.packageConfig.layer === "agent") {
      lines.push(`export type * from ${JSON.stringify(importPath)};`);
    } else {
      lines.push(`export type { ${list} } from ${JSON.stringify(importPath)};`);
    }
  }
  return lines.join("\n");
}

function generateTypeScript(record, model) {
  const sourceFiles = [record.manifestPath, record.schemaPath].map((file) => relative(repositoryRoot, file).replaceAll("\\", "/")).join(" and ");
  const imports = typescriptImports(record, model);
  const header = `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.\n// DO NOT EDIT MANUALLY.\n`;
  if (record.manifest.kind === "types") {
    return `${header}\n${imports ? `${imports}\n\n` : ""}${typeScriptDefinitions(record, model)}\n`;
  }
  if (record.manifest.transport.kind === "json-rpc-2.0") {
    const requestMap = record.manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.request, record.manifestPath, model)};`).join("\n");
    const responseMap = record.manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.response, record.manifestPath, model)};`).join("\n");
    const eventMap = record.manifest.events.map((event) => `  ${JSON.stringify(event.name)}: ${definitionName(event.payload, record.manifestPath, model)};`).join("\n");
    const methodNames = record.manifest.methods.map((method) => JSON.stringify(method.name)).join(", ");
    const eventNames = record.manifest.events.map((event) => JSON.stringify(event.name)).join(", ");
    const traceField = record.manifest.transport.traceContext?.field;
    const traceType = record.manifest.transport.traceContext
      ? referenceTarget(record.manifest.transport.traceContext.type, record.manifestPath, model, "TypeScript trace context").name
      : undefined;
    const requestTrace = traceField ? `; ${traceField}?: ${traceType}` : "";
    const eventTrace = requestTrace;
    const cursorField = record.manifest.transport.eventCursorField;
    const eventParams = cursorField
      ? `{ ${cursorField}: ${referenceTarget(record.manifest.transport.eventCursor, record.manifestPath, model, "TypeScript event cursor").name}; payload: ProtocolEventMap[E] }`
      : "ProtocolEventMap[E]";
    return `${header}
${imports ? `${imports}\n\n` : ""}export const PROTOCOL_VERSION = ${record.manifest.version} as const;
export const PROTOCOL_METHODS = [${methodNames}] as const;
export const PROTOCOL_EVENTS = [${eventNames}] as const;

${typeScriptDefinitions(record, model)}

export interface ProtocolRequestMap {
${requestMap}
}

export interface ProtocolResponseMap {
${responseMap}
}

export interface ProtocolEventMap {
${eventMap}
}

export type ProtocolMethod = keyof ProtocolRequestMap;
export type ProtocolEventName = keyof ProtocolEventMap;
export type ProtocolRequest = {
  [M in ProtocolMethod]: { jsonrpc: "2.0"; id: RequestId; method: M; params: ProtocolRequestMap[M]${requestTrace} }
}[ProtocolMethod];
export type ProtocolSuccessResponse<M extends ProtocolMethod = ProtocolMethod> = { jsonrpc: "2.0"; id: RequestId; result: ProtocolResponseMap[M] };
export type ProtocolErrorResponse = { jsonrpc: "2.0"; id: RequestId | null; error: RpcError };
export type ProtocolResponse<M extends ProtocolMethod = ProtocolMethod> = ProtocolSuccessResponse<M> | ProtocolErrorResponse;
export type ProtocolEvent = {
  [E in ProtocolEventName]: { jsonrpc: "2.0"; method: E; params: ${eventParams}${eventTrace} }
}[ProtocolEventName];

export interface ProtocolTransport {
  request<M extends ProtocolMethod>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]>;
}
`;
  }
  assert(record.manifest.transport.kind === "codepet-envelope", `${record.packageConfig.id} TypeScript generator currently supports codepet-envelope only`);
  const requestMap = record.manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.request, record.manifestPath, model)};`).join("\n");
  const responseMap = record.manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.response, record.manifestPath, model)};`).join("\n");
  const eventMap = record.manifest.events.map((event) => `  ${JSON.stringify(event.name)}: ${definitionName(event.payload, record.manifestPath, model)};`).join("\n");
  const requests = record.manifest.methods.map((method) => `  | { protocolVersion: ProtocolVersion; id: RequestId; method: ${JSON.stringify(method.name)}; params: ${definitionName(method.request, record.manifestPath, model)} }`).join("\n");
  const responses = record.manifest.methods.map((method) => `  | { protocolVersion: ProtocolVersion; id: RequestId; method: ${JSON.stringify(method.name)}; response: ResponsePayload<${definitionName(method.response, record.manifestPath, model)}> }`).join("\n");
  const cursorField = record.manifest.transport.eventCursorField;
  const cursorType = cursorField ? referenceTarget(record.manifest.transport.eventCursor, record.manifestPath, model, "TypeScript event cursor").name : undefined;
  const events = record.manifest.events.map((event) => {
    const cursor = cursorField ? `; ${cursorField}: ${cursorType}` : "";
    return `  | { protocolVersion: ProtocolVersion${cursor}; event: ${JSON.stringify(event.name)}; payload: ${definitionName(event.payload, record.manifestPath, model)} }`;
  }).join("\n");
  const clientMethods = record.manifest.methods.map((method) => `  ${camelCase(method.name)}(request: ${definitionName(method.request, record.manifestPath, model)}): Promise<${definitionName(method.response, record.manifestPath, model)}>;`).join("\n");
  const methodNames = record.manifest.methods.map((method) => JSON.stringify(method.name)).join(", ");
  const eventNames = record.manifest.events.map((event) => JSON.stringify(event.name)).join(", ");
  return `${header}
${imports ? `${imports}\n\n` : ""}export const PROTOCOL_VERSION = ${record.manifest.version} as const;
export const PROTOCOL_METHODS = [${methodNames}] as const;
export const PROTOCOL_EVENTS = [${eventNames}] as const;

${typeScriptDefinitions(record, model)}

export interface ProtocolRequestMap {
${requestMap}
}

export interface ProtocolResponseMap {
${responseMap}
}

export interface ProtocolEventMap {
${eventMap}
}

export type ProtocolMethod = keyof ProtocolRequestMap;
export type ProtocolEventName = keyof ProtocolEventMap;
export type ResponsePayload<T> = { status: "ok"; result: T } | { status: "error"; error: ProtocolError };

export type ProtocolRequest =
${requests};

export type ProtocolResponse =
${responses};

export type ProtocolEvent =
${events};

export interface ProtocolClient {
${clientMethods}
}

export interface ProtocolTransport {
  request<M extends ProtocolMethod>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]>;
}
`;
}

/**
 * Generator target adapter contract. A real adapter receives one validated
 * package record plus the complete protocol model and returns deterministic
 * source text. Planned adapters stay registered without a render function so
 * explicit selection fails before any output is written.
 *
 * render({ record, model, ir, output }) -> string
 */
export const generatorTargetRegistry = Object.freeze({
  rust: Object.freeze({
    id: "rust",
    interface: GENERATOR_TARGET_INTERFACE,
    implemented: true,
    render: ({ record, model, role }) => generateRust(record, model, role),
  }),
  typescript: Object.freeze({
    id: "typescript",
    interface: GENERATOR_TARGET_INTERFACE,
    implemented: true,
    render: ({ record, model }) => generateTypeScript(record, model),
  }),
  dart: Object.freeze({
    id: "dart",
    interface: GENERATOR_TARGET_INTERFACE,
    implemented: true,
    render: ({ record, model, ir }) => generateDart(record, model, ir),
  }),
  python: Object.freeze({
    id: "python",
    interface: GENERATOR_TARGET_INTERFACE,
    implemented: false,
  }),
});

export function resolveGeneratorTargets(targetIds, config) {
  assert(Array.isArray(targetIds) && targetIds.length > 0, "at least one generator target must be selected");
  assert(new Set(targetIds).size === targetIds.length, "generator targets must not contain duplicates");
  const declared = new Map(config.targets.map((target) => [target.id, target]));
  return targetIds.map((targetId) => {
    const adapter = generatorTargetRegistry[targetId];
    assert(adapter, `unknown generator target: ${targetId}`);
    const target = declared.get(targetId);
    assert(target, `generator target is not declared by protocol/codegen.json: ${targetId}`);
    assert(adapter.interface === target.interface, `generator target interface mismatch: ${targetId}`);
    assert(adapter.implemented && typeof adapter.render === "function", `generator target is not implemented: ${targetId}`);
    return adapter;
  });
}

async function updateGeneratedFile(path, content, checkMode, staleFiles) {
  let current;
  try {
    current = await readFile(path, "utf8");
  } catch {
    current = undefined;
  }
  if (current?.replaceAll("\r\n", "\n") === content.replaceAll("\r\n", "\n")) return;
  if (checkMode) {
    staleFiles.push(relative(outputRoot, path));
    return;
  }
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, content, "utf8");
}

export async function generateProtocol({ checkMode = false, targets = defaultTargetIds, config, role = "both" } = {}) {
  const model = await loadProtocolModel({ config });
  const adapters = resolveGeneratorTargets(targets, model.config);
  for (const adapter of adapters) {
    assert(
      model.records.some((record) => record.packageConfig.outputs[adapter.id]),
      `generator target has no package outputs: ${adapter.id}`,
    );
  }
  const staleFiles = [];
  const artifacts = [];
  for (const record of model.records) {
    for (const adapter of adapters) {
      const output = record.packageConfig.outputs[adapter.id];
      if (!output) continue;
      const content = adapter.render({ record, model, ir: model.protocolIr, output, role });
      artifacts.push({ packageId: record.packageConfig.id, targetId: adapter.id, output, content });
    }
  }
  for (const artifact of artifacts) {
    await updateGeneratedFile(resolve(outputRoot, artifact.output), artifact.content, checkMode, staleFiles);
  }
  if (staleFiles.length > 0) {
    fail(`generated protocol files are stale:\n${staleFiles.map((path) => `- ${path}`).join("\n")}\nRun npm run protocol:generate.`);
  }
  const generated = artifacts.map(({ content: _content, ...artifact }) => artifact);
  return { model, staleFiles, generated };
}

async function main() {
  const checkMode = process.argv.includes("--check");
  const obsoleteLanguageArgument = process.argv.find((argument) => argument.startsWith("--language="));
  assert(!obsoleteLanguageArgument, "--language is unsupported; use --target=<id>[,<id>]");
  const targetArgument = process.argv.find((argument) => argument.startsWith("--target="));
  const targets = targetArgument ? targetArgument.slice("--target=".length).split(",").filter(Boolean) : defaultTargetIds;
  await generateProtocol({ checkMode, targets });
  console.log(checkMode ? "protocol generated files are up to date" : "protocol generated files updated");
}

if (
  process.argv[1]
  && process.argv[1].endsWith("generate.mjs")
  && import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
