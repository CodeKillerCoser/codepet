import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import Ajv from "ajv/dist/2020.js";

// Discover schema and registry locations from the published SDK manifest.
// Communication validators are compiled from its sdkSchemaRoot entries; the
// plugin-install manifest remains a separate validation input.
const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "contract",
);
export const sdkManifest = JSON.parse(
  fs.readFileSync(path.join(root, "sdk-manifest.json")),
);
const read = (relative) =>
  JSON.parse(fs.readFileSync(path.join(root, relative)));
const ajv = new Ajv({ strict: false, allErrors: true });
const documents = new Map();
for (const value of Object.values(sdkManifest)) {
  if (
    typeof value !== "string" ||
    !value.startsWith(sdkManifest.sdkSchemaRoot) ||
    !value.endsWith(".schema.json")
  )
    continue;
  const schema = read(value);
  documents.set(path.resolve(root, value), schema);
  ajv.addSchema(schema);
}
export const methods = read(sdkManifest.methods).methods;
export const events = read(sdkManifest.events).events;
export const profiles = read(sdkManifest.profiles).profiles;
export const handshakeSchema = read(sdkManifest.transportHandshakeSchema);
export const schemaValidator = (entry, fragment = "") => {
  const schema = documents.get(path.resolve(root, entry));
  if (!schema) throw Error(`SDK schema not discovered: ${entry}`);
  const result = ajv.getSchema(schema.$id + fragment);
  if (!result)
    throw Error(`SDK schema reference unresolved: ${entry}${fragment}`);
  return result;
};
const registryValidator = (registry, ref) => {
  const hash = ref.indexOf("#"),
    file = hash < 0 ? ref : ref.slice(0, hash),
    fragment = hash < 0 ? "" : ref.slice(hash);
  return schemaValidator(
    path.relative(root, path.resolve(root, path.dirname(registry), file)),
    fragment,
  );
};
export const requestValidators = new Map(
  methods.map((m) => [
    m.name,
    registryValidator(sdkManifest.methods, m.request.$ref),
  ]),
);
export const responseValidators = new Map(
  methods.map((m) => [
    m.name,
    registryValidator(sdkManifest.methods, m.response.$ref),
  ]),
);
export const eventValidators = new Map(
  events.map((e) => [
    e.name,
    registryValidator(sdkManifest.events, e.payload.$ref),
  ]),
);
export const validateHello = schemaValidator(
  sdkManifest.transportHandshakeSchema,
  "#/$defs/ProviderTransportHello",
);
export const validateLimits = schemaValidator(
  sdkManifest.transportHandshakeSchema,
  "#/$defs/ProviderTransportLimits",
);
export const validatePluginManifest = new Ajv({
  strict: false,
  allErrors: true,
}).compile(read(sdkManifest.manifestSchema));
