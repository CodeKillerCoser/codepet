#!/usr/bin/env node

import { readFile, mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = resolve(repositoryRoot, "protocol/manifest.json");
const schemaPath = resolve(repositoryRoot, "protocol/schemas/v0.json");
const fixtureIndexPath = resolve(repositoryRoot, "protocol/fixtures/index.json");
const rustOutputPath = resolve(repositoryRoot, "src-tauri/src/runtime_gateway/generated.rs");
const typeScriptOutputPath = resolve(repositoryRoot, "frontend/lib/generated/runtimeGateway.ts");
const checkMode = process.argv.includes("--check");

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
    fail(`${path}: ${error.message}`);
  }
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function referenceName(reference, location) {
  assert(isObject(reference), `${location} must be an object containing $ref`);
  const keys = Object.keys(reference);
  assert(keys.length === 1 && keys[0] === "$ref", `${location} only supports $ref`);
  const match = reference.$ref.match(/^(?:\.\/schemas\/v0\.json)?#\/\$defs\/([A-Za-z][A-Za-z0-9]*)$/);
  assert(match, `${location} has an unsupported ref: ${reference.$ref}`);
  return match[1];
}

function resolveInternalRef(reference, schema, location) {
  const match = reference.match(/^#\/\$defs\/([A-Za-z][A-Za-z0-9]*)$/);
  assert(match, `${location} has an unsupported internal ref: ${reference}`);
  const target = schema.$defs[match[1]];
  assert(target, `${location} cannot resolve ${reference}`);
  return target;
}

function validateSchemaNode(node, schema, location) {
  assert(isObject(node), `${location} must be a schema object`);
  for (const keyword of Object.keys(node)) {
    assert(supportedKeywords.has(keyword), `${location} uses unsupported keyword ${keyword}`);
  }

  if (node.$ref !== undefined) {
    assert(Object.keys(node).length === 1, `${location} cannot combine $ref with other keywords`);
    resolveInternalRef(node.$ref, schema, location);
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
        validateSchemaNode(property, schema, `${location}.properties.${name}`);
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
    validateSchemaNode(node.items, schema, `${location}.items`);
  }
  for (const keyword of ["minimum", "maximum", "minLength", "minItems"]) {
    if (node[keyword] !== undefined) assert(Number.isInteger(node[keyword]) && node[keyword] >= 0, `${location}.${keyword} must be a non-negative integer`);
  }
  if (node.maximum !== undefined) {
    assert(node.maximum <= Number.MAX_SAFE_INTEGER, `${location}.maximum exceeds the JavaScript safe integer range`);
  }
  if (node.uniqueItems !== undefined) assert(typeof node.uniqueItems === "boolean", `${location}.uniqueItems must be boolean`);
}

function validateSchema(schema) {
  assert(isObject(schema), "protocol schema must be an object");
  assert(schema.$schema === "https://json-schema.org/draft/2020-12/schema", "protocol schema must use JSON Schema Draft 2020-12");
  assert(isObject(schema.$defs) && Object.keys(schema.$defs).length > 0, "protocol schema must define $defs");
  validateSchemaNode(schema, schema, "schema");
  for (const [name, definition] of Object.entries(schema.$defs)) {
    assert(/^[A-Z][A-Za-z0-9]*$/.test(name), `schema definition ${name} must be PascalCase`);
    validateSchemaNode(definition, schema, `schema.$defs.${name}`);
    assert(definition.$ref || definition.type, `schema.$defs.${name} must declare a type or ref`);
  }
}

function validateManifest(manifest, schema) {
  assert(isObject(manifest), "protocol manifest must be an object");
  assert(manifest.name === "code-pet-standard-protocol", "protocol manifest has an unexpected name");
  assert(Number.isInteger(manifest.version) && manifest.version >= 0, "protocol manifest version must be a non-negative integer");
  assert(Array.isArray(manifest.methods) && manifest.methods.length > 0, "protocol manifest methods must be non-empty");
  assert(Array.isArray(manifest.events) && manifest.events.length > 0, "protocol manifest events must be non-empty");

  const methodNames = new Set();
  for (const [index, method] of manifest.methods.entries()) {
    const location = `manifest.methods[${index}]`;
    assert(isObject(method), `${location} must be an object`);
    assert(/^[a-z][A-Za-z0-9]*\.[a-z][A-Za-z0-9]*$/.test(method.name), `${location}.name is invalid`);
    assert(!methodNames.has(method.name), `${location}.name duplicates ${method.name}`);
    methodNames.add(method.name);
    for (const side of ["request", "response"]) {
      const name = referenceName(method[side], `${location}.${side}`);
      const target = schema.$defs[name];
      assert(target, `${location}.${side} cannot resolve ${name}`);
      assert(target.type === "object" && target.properties !== undefined, `${location}.${side} must reference an object DTO`);
    }
  }

  const eventNames = new Set();
  for (const [index, event] of manifest.events.entries()) {
    const location = `manifest.events[${index}]`;
    assert(isObject(event), `${location} must be an object`);
    assert(/^[a-z][A-Za-z0-9]*\.[a-z][A-Za-z0-9]*$/.test(event.name), `${location}.name is invalid`);
    assert(!eventNames.has(event.name), `${location}.name duplicates ${event.name}`);
    eventNames.add(event.name);
    const name = referenceName(event.payload, `${location}.payload`);
    const target = schema.$defs[name];
    assert(target, `${location}.payload cannot resolve ${name}`);
    assert(target.type === "object" && target.properties !== undefined, `${location}.payload must reference an object DTO`);
  }

  const errorName = referenceName(manifest.error, "manifest.error");
  assert(schema.$defs[errorName]?.type === "object", "manifest.error must reference an object DTO");
}

function validateValue(value, node, schema, location) {
  if (node.$ref) return validateValue(value, resolveInternalRef(node.$ref, schema, location), schema, location);
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
    value.forEach((item, index) => validateValue(item, node.items, schema, `${location}[${index}]`));
    if (node.uniqueItems) assert(new Set(value.map((item) => JSON.stringify(item))).size === value.length, `${location} contains duplicate items`);
  } else if (node.type === "object") {
    assert(isObject(value), `${location} must be an object`);
    for (const required of node.required ?? []) assert(Object.hasOwn(value, required), `${location}.${required} is required`);
    for (const [name, item] of Object.entries(value)) {
      if (node.properties && Object.hasOwn(node.properties, name)) {
        validateValue(item, node.properties[name], schema, `${location}.${name}`);
      } else {
        assert(node.additionalProperties !== false, `${location}.${name} is not allowed`);
      }
    }
  }
}

function assertExactKeys(value, keys, location) {
  assert(isObject(value), `${location} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  assert(JSON.stringify(actual) === JSON.stringify(expected), `${location} keys must be ${expected.join(", ")}`);
}

async function validateFixtures(manifest, schema) {
  const fixtures = await readJson(fixtureIndexPath);
  assert(Array.isArray(fixtures) && fixtures.length > 0, "fixture index must be a non-empty array");
  for (const fixture of fixtures) {
    assert(isObject(fixture) && typeof fixture.file === "string" && typeof fixture.kind === "string", "fixture index entry is invalid");
    const value = await readJson(resolve(dirname(fixtureIndexPath), fixture.file));
    const location = `fixture ${fixture.file}`;
    assert(value.protocolVersion === manifest.version, `${location}.protocolVersion must equal manifest version`);
    if (fixture.kind === "request") {
      const method = manifest.methods.find((entry) => entry.name === fixture.name);
      assert(method, `${location} references unknown method ${fixture.name}`);
      assertExactKeys(value, ["protocolVersion", "id", "method", "params"], location);
      assert(typeof value.id === "string" && value.id.length > 0, `${location}.id is invalid`);
      assert(value.method === fixture.name, `${location}.method is invalid`);
      validateValue(value.params, schema.$defs[referenceName(method.request, location)], schema, `${location}.params`);
    } else if (fixture.kind === "response") {
      const method = manifest.methods.find((entry) => entry.name === fixture.name);
      assert(method, `${location} references unknown method ${fixture.name}`);
      assertExactKeys(value, ["protocolVersion", "id", "method", "response"], location);
      assert(value.method === fixture.name, `${location}.method is invalid`);
      assert(isObject(value.response), `${location}.response must be an object`);
      if (value.response.status === "ok") {
        assertExactKeys(value.response, ["status", "result"], `${location}.response`);
        validateValue(value.response.result, schema.$defs[referenceName(method.response, location)], schema, `${location}.response.result`);
      } else {
        assert(value.response.status === "error", `${location}.response.status is invalid`);
        assertExactKeys(value.response, ["status", "error"], `${location}.response`);
        validateValue(value.response.error, schema.$defs[referenceName(manifest.error, location)], schema, `${location}.response.error`);
      }
    } else if (fixture.kind === "event") {
      const event = manifest.events.find((entry) => entry.name === fixture.name);
      assert(event, `${location} references unknown event ${fixture.name}`);
      assertExactKeys(value, ["protocolVersion", "eventSequence", "event", "payload"], location);
      assert(Number.isSafeInteger(value.eventSequence) && value.eventSequence >= 0, `${location}.eventSequence is invalid`);
      assert(value.event === fixture.name, `${location}.event is invalid`);
      validateValue(value.payload, schema.$defs[referenceName(event.payload, location)], schema, `${location}.payload`);
    } else {
      fail(`${location} has unsupported kind ${fixture.kind}`);
    }
  }
}

function words(value) {
  return value
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean);
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

function definitionName(reference) {
  return referenceName(reference, "codegen reference");
}

function rustType(node, schema, definition) {
  if (node.$ref) return node.$ref.slice(node.$ref.lastIndexOf("/") + 1);
  if (node.type === "string") return "String";
  if (node.type === "boolean") return "bool";
  if (node.type === "integer") {
    if (definition === "ProtocolVersion") return "u32";
    return node.minimum !== undefined && node.minimum >= 0 ? "u64" : "i64";
  }
  if (node.type === "array") return `Vec<${rustType(node.items, schema)}>`;
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) {
    return "BTreeMap<String, serde_json::Value>";
  }
  fail(`cannot generate Rust type for ${JSON.stringify(node)}`);
}

function typeScriptType(node) {
  if (node.$ref) return node.$ref.slice(node.$ref.lastIndexOf("/") + 1);
  if (node.type === "string") return "string";
  if (node.type === "boolean") return "boolean";
  if (node.type === "integer") return "number";
  if (node.type === "array") return `Array<${typeScriptType(node.items)}>`;
  if (node.type === "object" && node.properties === undefined && node.additionalProperties === true) return "Record<string, JsonValue>";
  fail(`cannot generate TypeScript type for ${JSON.stringify(node)}`);
}

function rustDefinitions(schema) {
  const blocks = [];
  for (const name of Object.keys(schema.$defs).sort()) {
    const node = schema.$defs[name];
    if (node.enum) {
      const variants = node.enum.map((value) => `    #[serde(rename = "${value}")]\n    ${pascalCase(value)},`).join("\n");
      blocks.push(`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]\npub enum ${name} {\n${variants}\n}`);
    } else if (node.type === "object" && node.properties !== undefined) {
      const required = new Set(node.required ?? []);
      const fields = Object.entries(node.properties).map(([field, fieldSchema]) => {
        const baseType = rustType(fieldSchema, schema);
        const optional = !required.has(field);
        const attribute = optional ? "    #[serde(skip_serializing_if = \"Option::is_none\")]\n" : "";
        return `${attribute}    pub ${snakeCase(field)}: ${optional ? `Option<${baseType}>` : baseType},`;
      }).join("\n");
      const denyUnknown = node.additionalProperties === false ? "\n#[serde(deny_unknown_fields)]" : "";
      blocks.push(`#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(rename_all = "camelCase")]${denyUnknown}\npub struct ${name} {\n${fields}\n}`);
    } else {
      blocks.push(`pub type ${name} = ${rustType(node, schema, name)};`);
    }
  }
  return blocks.join("\n\n");
}

function typeScriptDefinitions(schema) {
  const blocks = ["export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };" ];
  for (const name of Object.keys(schema.$defs).sort()) {
    const node = schema.$defs[name];
    if (name === "JsonObject") {
      blocks.push("export type JsonObject = Record<string, JsonValue>;");
    } else if (node.enum) {
      blocks.push(`export type ${name} = ${node.enum.map((value) => JSON.stringify(value)).join(" | ")};`);
    } else if (node.type === "object" && node.properties !== undefined) {
      const required = new Set(node.required ?? []);
      const fields = Object.entries(node.properties).map(([field, fieldSchema]) => `  ${field}${required.has(field) ? "" : "?"}: ${typeScriptType(fieldSchema)};`).join("\n");
      blocks.push(`export interface ${name} {\n${fields}\n}`);
    } else {
      blocks.push(`export type ${name} = ${typeScriptType(node)};`);
    }
  }
  return blocks.join("\n\n");
}

function generateRust(manifest, schema) {
  const methodVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]\n    ${pascalCase(method.name)},`).join("\n");
  const methodAsStr = manifest.methods.map((method) => `            Self::${pascalCase(method.name)} => "${method.name}",`).join("\n");
  const requestVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]\n    ${pascalCase(method.name)} {\n        #[serde(rename = "protocolVersion")]\n        protocol_version: ProtocolVersion,\n        id: RequestId,\n        params: ${definitionName(method.request)},\n    },`).join("\n");
  const responseVariants = manifest.methods.map((method) => `    #[serde(rename = "${method.name}")]\n    ${pascalCase(method.name)} {\n        #[serde(rename = "protocolVersion")]\n        protocol_version: ProtocolVersion,\n        id: RequestId,\n        response: ResponsePayload<${definitionName(method.response)}>,\n    },`).join("\n");
  const eventVariants = manifest.events.map((event) => `    #[serde(rename = "${event.name}")]\n    ${pascalCase(event.name)} {\n        #[serde(rename = "protocolVersion")]\n        protocol_version: ProtocolVersion,\n        #[serde(rename = "eventSequence")]\n        event_sequence: EventSequence,\n        payload: ${definitionName(event.payload)},\n    },`).join("\n");
  const traitMethods = manifest.methods.map((method) => `    fn ${snakeCase(method.name)}<'a>(&'a self, _request: ${definitionName(method.request)}) -> ProtocolFuture<'a, ${definitionName(method.response)}> {\n        Box::pin(async { Err(method_not_implemented("${method.name}")) })\n    }`).join("\n\n");
  const dispatchArms = manifest.methods.map((method) => {
    const variant = pascalCase(method.name);
    const handler = snakeCase(method.name);
    return `        ProtocolRequest::${variant} { protocol_version, id, params } => {\n            let response = match server.${handler}(params).await {\n                Ok(result) => ResponsePayload::Ok { result },\n                Err(error) => ResponsePayload::Error { error },\n            };\n            ProtocolResponse::${variant} { protocol_version, id, response }\n        }`;
  }).join(",\n");

  return `// @generated by scripts/generate_protocol.mjs from protocol/manifest.json and protocol/schemas/v0.json.\n// DO NOT EDIT MANUALLY.\n\nuse serde::{Deserialize, Serialize};\nuse std::collections::BTreeMap;\nuse std::future::Future;\nuse std::pin::Pin;\n\npub const PROTOCOL_VERSION: ProtocolVersion = ${manifest.version};\n\n${rustDefinitions(schema)}\n\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]\npub enum ProtocolMethod {\n${methodVariants}\n}\n\nimpl ProtocolMethod {\n    pub const fn as_str(self) -> &'static str {\n        match self {\n${methodAsStr}\n        }\n    }\n}\n\n#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(tag = "status", rename_all = "camelCase")]\npub enum ResponsePayload<T> {\n    Ok { result: T },\n    Error { error: ProtocolError },\n}\n\n#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(tag = "method")]\npub enum ProtocolRequest {\n${requestVariants}\n}\n\n#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(tag = "method")]\npub enum ProtocolResponse {\n${responseVariants}\n}\n\n#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(tag = "event")]\npub enum ProtocolEvent {\n${eventVariants}\n}\n\npub type ProtocolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProtocolError>> + Send + 'a>>;\n\npub trait ProtocolServer: Send + Sync {\n${traitMethods}\n}\n\nfn method_not_implemented(method: &str) -> ProtocolError {\n    ProtocolError {\n        code: "method_not_implemented".to_string(),\n        message: format!("protocol method is not implemented: {method}"),\n        retryable: false,\n        details: None,\n    }\n}\n\npub async fn dispatch<S: ProtocolServer + ?Sized>(server: &S, request: ProtocolRequest) -> ProtocolResponse {\n    match request {\n${dispatchArms}\n    }\n}\n\npub struct ProtocolDispatcher<S> {\n    server: S,\n}\n\nimpl<S> ProtocolDispatcher<S> {\n    pub const fn new(server: S) -> Self {\n        Self { server }\n    }\n\n    pub const fn server(&self) -> &S {\n        &self.server\n    }\n}\n\nimpl<S: ProtocolServer> ProtocolDispatcher<S> {\n    pub async fn dispatch(&self, request: ProtocolRequest) -> ProtocolResponse {\n        dispatch(&self.server, request).await\n    }\n}\n`;
}

function generateTypeScript(manifest, schema) {
  const requestMap = manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.request)};`).join("\n");
  const responseMap = manifest.methods.map((method) => `  ${JSON.stringify(method.name)}: ${definitionName(method.response)};`).join("\n");
  const eventMap = manifest.events.map((event) => `  ${JSON.stringify(event.name)}: ${definitionName(event.payload)};`).join("\n");
  const requests = manifest.methods.map((method) => `  | { protocolVersion: ProtocolVersion; id: RequestId; method: ${JSON.stringify(method.name)}; params: ${definitionName(method.request)} }`).join("\n");
  const responses = manifest.methods.map((method) => `  | { protocolVersion: ProtocolVersion; id: RequestId; method: ${JSON.stringify(method.name)}; response: ResponsePayload<${definitionName(method.response)}> }`).join("\n");
  const events = manifest.events.map((event) => `  | { protocolVersion: ProtocolVersion; eventSequence: EventSequence; event: ${JSON.stringify(event.name)}; payload: ${definitionName(event.payload)} }`).join("\n");
  const clientMethods = manifest.methods.map((method) => `  ${camelCase(method.name)}(request: ${definitionName(method.request)}): Promise<${definitionName(method.response)}>;`).join("\n");
  const methodNames = manifest.methods.map((method) => JSON.stringify(method.name)).join(", ");
  const eventNames = manifest.events.map((event) => JSON.stringify(event.name)).join(", ");

  return `// @generated by scripts/generate_protocol.mjs from protocol/manifest.json and protocol/schemas/v0.json.\n// DO NOT EDIT MANUALLY.\n\nexport const PROTOCOL_VERSION = ${manifest.version} as const;\nexport const PROTOCOL_METHODS = [${methodNames}] as const;\nexport const PROTOCOL_EVENTS = [${eventNames}] as const;\n\n${typeScriptDefinitions(schema)}\n\nexport interface ProtocolRequestMap {\n${requestMap}\n}\n\nexport interface ProtocolResponseMap {\n${responseMap}\n}\n\nexport interface ProtocolEventMap {\n${eventMap}\n}\n\nexport type ProtocolMethod = keyof ProtocolRequestMap;\nexport type ProtocolEventName = keyof ProtocolEventMap;\nexport type ResponsePayload<T> = { status: "ok"; result: T } | { status: "error"; error: ProtocolError };\n\nexport type ProtocolRequest =\n${requests};\n\nexport type ProtocolResponse =\n${responses};\n\nexport type ProtocolEvent =\n${events};\n\nexport interface ProtocolClient {\n${clientMethods}\n}\n\nexport interface ProtocolTransport {\n  request<M extends ProtocolMethod>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]>;\n}\n`;
}

async function updateGeneratedFile(path, content, staleFiles) {
  let current;
  try {
    current = await readFile(path, "utf8");
  } catch {
    current = undefined;
  }
  if (current === content) return;
  if (checkMode) {
    staleFiles.push(path.slice(repositoryRoot.length + 1));
    return;
  }
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, content, "utf8");
}

async function main() {
  const [manifest, schema] = await Promise.all([readJson(manifestPath), readJson(schemaPath)]);
  validateSchema(schema);
  validateManifest(manifest, schema);
  await validateFixtures(manifest, schema);

  const staleFiles = [];
  await updateGeneratedFile(rustOutputPath, generateRust(manifest, schema), staleFiles);
  await updateGeneratedFile(typeScriptOutputPath, generateTypeScript(manifest, schema), staleFiles);

  if (staleFiles.length > 0) {
    fail(`generated protocol files are stale:\n${staleFiles.map((path) => `- ${path}`).join("\n")}\nRun npm run protocol:generate.`);
  }
  console.log(checkMode ? "protocol generated files are up to date" : "protocol generated files updated");
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
