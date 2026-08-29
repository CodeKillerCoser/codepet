#!/usr/bin/env node

import { readFile, mkdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const protocolRoot = resolve(repositoryRoot, "protocol");
const configPath = resolve(protocolRoot, "codegen.json");

const supportedKeywords = new Set([
  "$schema",
  "$id",
  "$defs",
  "$ref",
  "title",
  "description",
  "type",
  "enum",
  "properties",
  "required",
  "additionalProperties",
  "items",
  "minimum",
  "maximum",
  "minLength",
  "minItems",
  "uniqueItems",
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

  if (node.type !== undefined) {
    assert(["object", "array", "string", "integer", "boolean"].includes(node.type), `${location} has unsupported type ${node.type}`);
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
        assert(/^[a-z][A-Za-z0-9]*$/.test(name), `${location} property ${name} must be camelCase`);
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
      assert(typeof node.additionalProperties === "boolean", `${location}.additionalProperties must be boolean`);
    }
  }
  if (node.type === "array") {
    assert(node.items !== undefined, `${location}.items is required for arrays`);
    validateSchemaNode(node.items, record, model, `${location}.items`);
  }
  for (const keyword of ["minimum", "maximum", "minLength", "minItems"]) {
    if (node[keyword] !== undefined) {
      assert(Number.isSafeInteger(node[keyword]), `${location}.${keyword} must be a safe integer`);
      if (keyword === "minLength" || keyword === "minItems") {
        assert(node[keyword] >= 0, `${location}.${keyword} must be non-negative`);
      }
    }
  }
  if (node.minimum !== undefined && node.maximum !== undefined) {
    assert(node.minimum <= node.maximum, `${location}.minimum exceeds maximum`);
  }
  if (node.uniqueItems !== undefined) {
    assert(typeof node.uniqueItems === "boolean", `${location}.uniqueItems must be boolean`);
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
    assert(definition.$ref || definition.type, `${packageConfig.id} definition ${name} must declare a type or ref`);
    validateSchemaNode(definition, record, model, `${packageConfig.id}.schema.$defs.${name}`);
  }

  const actualDependencies = new Set();
  visitRefs(schema, (reference) => {
    const target = parseReference(reference, schemaPath, model, `${packageConfig.id}.schema`);
    if (target.record.packageConfig.id !== packageConfig.id) actualDependencies.add(target.record.packageConfig.id);
  });
  const declaredDependencies = new Set(packageConfig.dependencies);
  for (const dependency of actualDependencies) {
    assert(declaredDependencies.has(dependency), `${packageConfig.id} uses undeclared dependency ${dependency}`);
  }
  for (const dependency of declaredDependencies) {
    assert(actualDependencies.has(dependency) || packageConfig.id === "gateway-compat-v0", `${packageConfig.id} declares unused dependency ${dependency}`);
  }
  if (packageConfig.layer === "core") {
    assert(actualDependencies.size === 0, "core-v1 must not depend on pet, provider, or gateway schemas");
  } else {
    assert(
      [...actualDependencies].every((id) => id === "core-v1"),
      `${packageConfig.id} may only reference core-v1`,
    );
  }
}

function validateManifest(record, model) {
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
    return;
  }

  assert(isObject(manifest.transport), `${packageConfig.id} service manifest requires transport`);
  assert(["codepet-envelope", "json-rpc-2.0"].includes(manifest.transport.kind), `${packageConfig.id} has unsupported transport kind`);
  assert(manifest.transport.requestDiscriminator === "method", `${packageConfig.id} request discriminator must be method`);
  assert(typeof manifest.transport.framing === "string" && manifest.transport.framing.length > 0, `${packageConfig.id} transport framing is required`);
  const requestId = referenceTarget(manifest.transport.requestId, manifestPath, model, `${packageConfig.id}.transport.requestId`);
  assert(requestId.name === "RequestId", `${packageConfig.id} transport requestId must reference core RequestId`);
  if (manifest.transport.kind === "json-rpc-2.0") {
    assert(manifest.transport.jsonRpcVersion === "2.0", `${packageConfig.id} must use JSON-RPC 2.0`);
    assert(manifest.transport.framing === "stdio-json-lines", `${packageConfig.id} provider framing must be stdio-json-lines`);
    assert(manifest.transport.eventDiscriminator === "method", `${packageConfig.id} JSON-RPC event discriminator must be method`);
    const rpcError = referenceTarget(manifest.transport.rpcError, manifestPath, model, `${packageConfig.id}.transport.rpcError`);
    assert(rpcError.name === "RpcError", `${packageConfig.id} transport rpcError must reference core RpcError`);
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
    assert(/^[a-z][A-Za-z0-9]*\.[a-z][A-Za-z0-9]*$/.test(method.name), `${location}.name is invalid`);
    assert(!methodNames.has(method.name), `${location}.name duplicates ${method.name}`);
    methodNames.add(method.name);
    assert(typeof method.direction === "string" && method.direction.length > 0, `${location}.direction is required`);
    assert(["safe", "idempotent", "nonIdempotent"].includes(method.idempotency), `${location}.idempotency is invalid`);
    for (const side of ["request", "response"]) {
      const target = referenceTarget(method[side], manifestPath, model, `${location}.${side}`);
      assert(target.node.type === "object" && target.node.properties !== undefined, `${location}.${side} must reference an object DTO`);
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

function validateValue(value, node, record, model, location) {
  if (node.$ref) {
    const target = parseReference(node.$ref, record.schemaPath, model, location);
    return validateValue(value, target.node, target.record, model, location);
  }
  if (node.enum) assert(node.enum.includes(value), `${location} must be one of ${node.enum.join(", ")}`);
  if (node.type === "string") {
    assert(typeof value === "string", `${location} must be a string`);
    if (node.minLength !== undefined) assert(value.length >= node.minLength, `${location} is too short`);
  } else if (node.type === "integer") {
    assert(Number.isSafeInteger(value), `${location} must be a safe integer`);
    if (node.minimum !== undefined) assert(value >= node.minimum, `${location} is below minimum`);
    if (node.maximum !== undefined) assert(value <= node.maximum, `${location} is above maximum`);
  } else if (node.type === "boolean") {
    assert(typeof value === "boolean", `${location} must be boolean`);
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
    const value = await readJson(resolve(dirname(fixtureIndexPath), fixture.file));
    const location = `${packageConfig.id} fixture ${fixture.file}`;
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
        const target = referenceTarget(event.payload, manifestPath, model, `${location}.params`);
        validateValue(value.params, target.node, target.record, model, `${location}.params`);
      } else {
        fail(`${location} has unsupported kind ${fixture.kind}`);
      }
    }
  }
}

export async function loadProtocolModel() {
  const config = await readJson(configPath);
  assert(config.generatorInterfaceVersion === 1, "protocol/codegen.json has unsupported generatorInterfaceVersion");
  assert(Array.isArray(config.languages) && config.languages.length >= 4, "protocol/codegen.json must declare language targets");
  const languageIds = new Set(config.languages.map((language) => language.id));
  for (const required of ["rust", "typescript", "dart", "python"]) {
    assert(languageIds.has(required), `protocol/codegen.json is missing ${required} generator interface`);
  }
  assert(Array.isArray(config.packages) && config.packages.length > 0, "protocol/codegen.json packages must be non-empty");

  const packageIds = new Set();
  const records = [];
  const schemasByPath = new Map();
  for (const packageConfig of config.packages) {
    assert(isObject(packageConfig), "protocol package entry must be an object");
    assert(!packageIds.has(packageConfig.id), `duplicate protocol package ${packageConfig.id}`);
    packageIds.add(packageConfig.id);
    assert(["core", "pet", "provider", "gateway"].includes(packageConfig.layer), `${packageConfig.id} has invalid layer`);
    assert(Number.isInteger(packageConfig.version) && packageConfig.version >= 0, `${packageConfig.id} has invalid version`);
    assert(Array.isArray(packageConfig.dependencies), `${packageConfig.id} dependencies must be an array`);
    assert(isObject(packageConfig.outputs) && Object.keys(packageConfig.outputs).length > 0, `${packageConfig.id} outputs must be non-empty`);
    for (const language of Object.keys(packageConfig.outputs)) {
      assert(languageIds.has(language), `${packageConfig.id} uses undeclared language ${language}`);
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
  for (const record of records) await validateFixtures(record, model);
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
  if (node.type === "string") return "String";
  if (node.type === "boolean") return "bool";
  if (node.type === "integer") {
    if (definition === "ProtocolVersion") return "u32";
    return node.minimum !== undefined && node.minimum >= 0 ? "u64" : "i64";
  }
  if (node.type === "array") return `Vec<${rustType(node.items, record, model)}>`;
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) {
    return "BTreeMap<String, serde_json::Value>";
  }
  fail(`cannot generate Rust type for ${record.packageConfig.id}: ${JSON.stringify(node)}`);
}

function typeScriptType(node, record, model) {
  if (node.$ref) return parseReference(node.$ref, record.schemaPath, model, "TypeScript type").name;
  if (node.type === "string") return "string";
  if (node.type === "boolean") return "boolean";
  if (node.type === "integer") return "number";
  if (node.type === "array") return `Array<${typeScriptType(node.items, record, model)}>`;
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) return "Record<string, JsonValue>";
  fail(`cannot generate TypeScript type for ${record.packageConfig.id}: ${JSON.stringify(node)}`);
}

function externalReferences(record, model) {
  const byPackage = new Map();
  const add = (reference, sourcePath) => {
    const target = parseReference(reference, sourcePath, model, `${record.packageConfig.id} external import`);
    if (target.record.packageConfig.id === record.packageConfig.id) return;
    if (!byPackage.has(target.record.packageConfig.id)) byPackage.set(target.record.packageConfig.id, new Set());
    byPackage.get(target.record.packageConfig.id).add(target.name);
  };
  visitRefs(record.schema, (reference) => add(reference, record.schemaPath));
  visitRefs(record.manifest, (reference) => add(reference, record.manifestPath));
  return byPackage;
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
    const crate = rustDependencyCrate(model.recordsById.get(packageId));
    imports.push(`pub use ${crate}::{${[...names].sort().join(", ")}};`);
  }
  return imports.join("\n");
}

function rustDefinitions(record, model) {
  const blocks = [];
  for (const name of Object.keys(record.schema.$defs).sort()) {
    const node = record.schema.$defs[name];
    if (node.enum) {
      const variants = node.enum.map((value) => `    #[serde(rename = "${value}")]\n    ${pascalCase(value)},`).join("\n");
      blocks.push(`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]\npub enum ${name} {\n${variants}\n}`);
    } else if (node.type === "object" && node.properties !== undefined) {
      const required = new Set(node.required ?? []);
      const fields = Object.entries(node.properties).map(([field, fieldSchema]) => {
        const baseType = rustType(fieldSchema, record, model);
        const optional = !required.has(field);
        const attribute = optional ? "    #[serde(skip_serializing_if = \"Option::is_none\")]\n" : "";
        return `${attribute}    pub ${snakeCase(field)}: ${optional ? `Option<${baseType}>` : baseType},`;
      }).join("\n");
      const denyUnknown = node.additionalProperties === false ? "\n#[serde(deny_unknown_fields)]" : "";
      blocks.push(`#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(rename_all = "camelCase")]${denyUnknown}\npub struct ${name} {\n${fields}\n}`);
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
  const eventVariants = record.manifest.events.map((event) => `    #[serde(rename = "${event.name}")]\n    ${pascalCase(event.name)},`).join("\n");
  const eventAsStr = record.manifest.events.map((event) => `            Self::${pascalCase(event.name)} => "${event.name}",`).join("\n");
  const eventFromStr = record.manifest.events.map((event) => `            "${event.name}" => Ok(Self::${pascalCase(event.name)}),`).join("\n");
  return `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolMethod {
${methodVariants}
}

impl ProtocolMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
${methodAsStr}
        }
    }
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
  return `pub type ProtocolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProtocolError>> + Send + 'a>>;

pub trait ProtocolServer: Send + Sync {
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
  return `pub type ProtocolTransportFuture<'a> = Pin<Box<dyn Future<Output = Result<serde_json::Value, ProtocolError>> + Send + 'a>>;

pub trait ProtocolTransport: Send + Sync {
    fn request<'a>(&'a self, method: ProtocolMethod, params: serde_json::Value) -> ProtocolTransportFuture<'a>;
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
${methods}
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

function generateCodepetEnvelope(record, model) {
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

${serverTrait(record, model)}

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
}

${clientSupport(record, model)}

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
}`;
}

function generateJsonRpc(record, model) {
  const { manifest, manifestPath } = record;
  const requestVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]
    ${pascalCase(method.name)} {
        jsonrpc: String,
        id: RequestId,
        params: ${definitionName(method.request, manifestPath, model)},
    },`).join("\n");
  const eventVariants = manifest.events.map((event) => `    #[serde(rename = "${event.name}")]
    ${pascalCase(event.name)} {
        jsonrpc: String,
        params: ${definitionName(event.payload, manifestPath, model)},
    },`).join("\n");
  const requestVersions = manifest.methods.map((method) => `            Self::${pascalCase(method.name)} { jsonrpc, .. } => jsonrpc,`).join("\n");
  const eventVersions = manifest.events.map((event) => `            Self::${pascalCase(event.name)} { jsonrpc, .. } => jsonrpc,`).join("\n");
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
            JsonRpcResponse { jsonrpc, id, response }
        }`;
  }).join(",\n");
  return `${protocolEnums(record, model)}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.requestDiscriminator}")]
pub enum ProtocolRequest {
${requestVariants}
}

impl ProtocolRequest {
    pub fn jsonrpc_version(&self) -> &str {
        match self {
${requestVersions}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "${manifest.transport.eventDiscriminator}")]
pub enum ProtocolEvent {
${eventVariants}
}

impl ProtocolEvent {
    pub fn jsonrpc_version(&self) -> &str {
        match self {
${eventVersions}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponsePayload {
    Ok { result: serde_json::Value },
    Error { error: RpcError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(flatten)]
    pub response: JsonRpcResponsePayload,
}

${serverTrait(record, model)}

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
}

${clientSupport(record, model)}

fn rpc_method_error(error: ProtocolError) -> RpcError {
    let message = error.message.clone();
    let data = serde_json::to_value(error).ok().and_then(|value| match value {
        serde_json::Value::Object(entries) => Some(entries.into_iter().collect()),
        _ => None,
    });
    RpcError { code: -32000, message, data }
}

fn rpc_codec_error(context: &str, error: serde_json::Error) -> RpcError {
    RpcError { code: -32603, message: format!("{context}: {error}"), data: None }
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

pub fn encode_request(value: &ProtocolRequest) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(value.jsonrpc_version())?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode request", error))
}

pub fn decode_request(value: &[u8]) -> Result<ProtocolRequest, ProtocolError> {
    let request: ProtocolRequest = serde_json::from_slice(value).map_err(|error| codec_error("decode request", error))?;
    validate_jsonrpc(request.jsonrpc_version())?;
    Ok(request)
}

pub fn encode_response(value: &JsonRpcResponse) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(&value.jsonrpc)?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode response", error))
}

pub fn decode_response(value: &[u8]) -> Result<JsonRpcResponse, ProtocolError> {
    let response: JsonRpcResponse = serde_json::from_slice(value).map_err(|error| codec_error("decode response", error))?;
    validate_jsonrpc(&response.jsonrpc)?;
    Ok(response)
}

pub fn encode_event(value: &ProtocolEvent) -> Result<Vec<u8>, ProtocolError> {
    validate_jsonrpc(value.jsonrpc_version())?;
    serde_json::to_vec(value).map_err(|error| codec_error("encode event", error))
}

pub fn decode_event(value: &[u8]) -> Result<ProtocolEvent, ProtocolError> {
    let event: ProtocolEvent = serde_json::from_slice(value).map_err(|error| codec_error("decode event", error))?;
    validate_jsonrpc(event.jsonrpc_version())?;
    Ok(event)
}`;
}

function generateRust(record, model) {
  const sourceFiles = [relative(repositoryRoot, record.manifestPath), relative(repositoryRoot, record.schemaPath)].join(" and ");
  const imports = rustImports(record, model);
  const definitions = rustDefinitions(record, model);
  const collectionsImport = definitions.includes("BTreeMap<") ? "use std::collections::BTreeMap;\n" : "";
  if (record.manifest.kind === "types") {
    return `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.
// DO NOT EDIT MANUALLY.

use serde::{Deserialize, Serialize};
${collectionsImport}

${imports ? `${imports}\n\n` : ""}${definitions}
`;
  }
  const service = record.manifest.transport.kind === "json-rpc-2.0"
    ? generateJsonRpc(record, model)
    : generateCodepetEnvelope(record, model);
  return `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.
// DO NOT EDIT MANUALLY.

use serde::{Deserialize, Serialize};
${collectionsImport}use std::future::Future;
use std::pin::Pin;

${imports ? `${imports}\n\n` : ""}pub const PROTOCOL_VERSION: ProtocolVersion = ${record.manifest.version};

${definitions}

${service}
`;
}

function typeScriptDefinitions(record, model) {
  const blocks = ["export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };"];
  for (const name of Object.keys(record.schema.$defs).sort()) {
    const node = record.schema.$defs[name];
    if (node.enum) {
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
  const outputPath = resolve(repositoryRoot, record.packageConfig.outputs.typescript);
  const lines = [];
  for (const [packageId, names] of [...externalReferences(record, model)].sort(([left], [right]) => left.localeCompare(right))) {
    const target = model.recordsById.get(packageId);
    const targetOutput = target.packageConfig.outputs.typescript;
    assert(targetOutput, `${record.packageConfig.id} needs TypeScript output from ${packageId}`);
    let importPath = relative(dirname(outputPath), resolve(repositoryRoot, targetOutput)).replaceAll("\\", "/").replace(/\.ts$/, "");
    if (!importPath.startsWith(".")) importPath = `./${importPath}`;
    const list = [...names].sort().join(", ");
    lines.push(`import type { ${list} } from ${JSON.stringify(importPath)};`);
    lines.push(`export type { ${list} } from ${JSON.stringify(importPath)};`);
  }
  return lines.join("\n");
}

function generateTypeScript(record, model) {
  const sourceFiles = [relative(repositoryRoot, record.manifestPath), relative(repositoryRoot, record.schemaPath)].join(" and ");
  const imports = typescriptImports(record, model);
  const header = `// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.\n// DO NOT EDIT MANUALLY.\n`;
  if (record.manifest.kind === "types") {
    return `${header}\n${imports ? `${imports}\n\n` : ""}${typeScriptDefinitions(record, model)}\n`;
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

async function updateGeneratedFile(path, content, checkMode, staleFiles) {
  let current;
  try {
    current = await readFile(path, "utf8");
  } catch {
    current = undefined;
  }
  if (current === content) return;
  if (checkMode) {
    staleFiles.push(relative(repositoryRoot, path));
    return;
  }
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, content, "utf8");
}

export async function generateProtocol({ checkMode = false, languages = ["rust", "typescript"] } = {}) {
  const model = await loadProtocolModel();
  const knownLanguages = new Set(model.config.languages.map((language) => language.id));
  for (const language of languages) assert(knownLanguages.has(language), `unknown language target: ${language}`);
  const staleFiles = [];
  for (const record of model.records) {
    for (const language of languages) {
      const output = record.packageConfig.outputs[language];
      if (!output) continue;
      const content = language === "rust" ? generateRust(record, model) : generateTypeScript(record, model);
      await updateGeneratedFile(resolve(repositoryRoot, output), content, checkMode, staleFiles);
    }
  }
  if (staleFiles.length > 0) {
    fail(`generated protocol files are stale:\n${staleFiles.map((path) => `- ${path}`).join("\n")}\nRun npm run protocol:generate.`);
  }
  return { model, staleFiles };
}

async function main() {
  const checkMode = process.argv.includes("--check");
  const languageArgument = process.argv.find((argument) => argument.startsWith("--language="));
  const languages = languageArgument ? languageArgument.slice("--language=".length).split(",").filter(Boolean) : ["rust", "typescript"];
  await generateProtocol({ checkMode, languages });
  console.log(checkMode ? "protocol generated files are up to date" : "protocol generated files updated");
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
