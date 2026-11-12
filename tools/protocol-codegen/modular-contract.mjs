// Canonical v1 module graph. Old package paths and the public SDK are derived views.
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';

const encode = value => `${JSON.stringify(value, null, 2)}\n`;
const assert = (value, message) => { if (!value) throw new Error(`Protocol IDL: ${message}`); };
const resolveFile = (source, relative) => path.posix.normalize(path.posix.join(path.posix.dirname(source), relative));
const schemaKeys = ['$defs', 'properties', 'patternProperties'];
export function validateDescriptions(value, location) {
  assert(typeof value.description === 'string' && value.description.trim(), `missing description at ${location}`);
  for (const key of schemaKeys) for (const [name, child] of Object.entries(value[key] ?? {})) validateDescriptions(child, `${location}/${key}/${name}`);
  for (const key of ['items', 'additionalProperties', 'not', 'if', 'then', 'else']) if (value[key] && typeof value[key] === 'object') validateDescriptions(value[key], `${location}/${key}`);
  for (const key of ['oneOf', 'anyOf', 'allOf', 'prefixItems']) for (const [i, child] of (value[key] ?? []).entries()) validateDescriptions(child, `${location}/${key}/${i}`);
}
function walk(value, callback) {
  if (!value || typeof value !== 'object') return;
  callback(value);
  for (const child of Object.values(value)) if (Array.isArray(child)) child.forEach(v => walk(v, callback)); else walk(child, callback);
}
function targetOf(ref, source) {
  const [relative, fragment = ''] = ref.split('#');
  assert(!/^[a-z]+:/i.test(relative), `external reference ${ref}`);
  return { file: relative ? resolveFile(source, relative) : source, fragment };
}

export async function loadModularContract(protocolRoot) {
  const root = path.join(protocolRoot, 'v1');
  const files = new Map(), layers = new Map(), schemaLayers = new Map();
  async function read(file, layer) {
    assert(!path.posix.isAbsolute(file) && !file.startsWith('../'), `source escapes v1: ${file}`);
    assert(!files.has(file), `source is registered twice: ${file}`);
    const value = JSON.parse(await readFile(path.join(root, file), 'utf8'));
    files.set(file, value); layers.set(file, layer);
    if (file.endsWith('.schema.json')) schemaLayers.set(file, layer);
    return value;
  }
  const index = await read('protocol.json', 'index');
  assert(index.version === 1, 'unsupported source version');
  for (const key of ['commonSchema', 'rpcEnvelopeSchema']) await read(index[key], 'common');
  await read(index.agentDomainSchema, 'agent');
  async function service(file, layer) {
    const entry = await read(file, layer), modules = [], moduleIds = new Set();
    assert(entry.version === index.version, `${file}: inconsistent protocol version`);
    assert(!entry.service.methods && !entry.service.events, `${file}: methods/events belong to modules`);
    for (const module of entry.modules) {
      assert(module.id && !moduleIds.has(module.id), `${file}: duplicate module ${module.id}`);
      moduleIds.add(module.id);
      const moduleLayer = layer === 'gateway' && module.id === 'pairing' ? 'pairing' : layer;
      const record = { ...module, layer: moduleLayer };
      for (const key of ['schema', 'methods', 'events', 'bindings', 'service']) if (module[key]) {
        record[`${key}File`] = resolveFile(file, module[key]);
        record[key] = await read(record[`${key}File`], moduleLayer);
      }
      assert(record.schema?.$defs, `${file}: module ${module.id} requires a Schema`);
      modules.push(record);
    }
    for (const kind of ['methods', 'events']) {
      const seen = new Set();
      for (const module of modules) for (const item of module[kind]?.[kind] ?? []) {
        assert(item.description?.trim(), `${module[`${kind}File`]}: missing description for ${item.name}`);
        assert(!seen.has(item.name), `${layer}: duplicate ${kind} name ${item.name}`);
        seen.add(item.name);
      }
    }
    return { entry, file, modules };
  }
  const provider = await service(index.provider, 'provider');
  for (const key of ['manifestSchema', 'profiles', 'transportHandshakeSchema', 'sdkManifest']) {
    provider[`${key}File`] = resolveFile(provider.file, provider.entry[key]);
    provider[key] = await read(provider[`${key}File`], key === 'sdkManifest' ? 'metadata' : 'provider');
  }
  assert(provider.sdkManifest.protocolVersion === provider.entry.version, 'SDK metadata and Provider protocol versions disagree');
  const gateway = await service(index.gateway, 'gateway');
  const heartbeat = gateway.modules.find(module => module.id === 'session')?.schema['x-codepet-lifecycle'];
  assert(heartbeat && Object.values(heartbeat).every(value => Number.isSafeInteger(value) && value > 0)
    && heartbeat.heartbeatIntervalMs < heartbeat.heartbeatTimeoutMs
    && heartbeat.heartbeatRequestTimeoutMs < heartbeat.heartbeatTimeoutMs, 'invalid Gateway heartbeat policy');
  const control = { file: index.channelControl };
  control.entry = await read(control.file, 'control');
  control.schemaFile = resolveFile(control.file, control.entry.schema);
  control.schema = await read(control.schemaFile, 'control');

  const allowed = { common: ['common'], agent: ['common', 'agent'], provider: ['common', 'agent', 'provider'], gateway: ['common', 'agent', 'gateway'], pairing: ['common', 'pairing'], control: ['common', 'control'] };
  for (const [file, document] of files) {
    if (file.endsWith('.schema.json')) validateDescriptions(document, file);
    walk(document, node => {
      if (!node.$ref) return;
      const target = targetOf(node.$ref, file);
      assert(files.has(target.file), `missing reference file ${target.file} in ${file}`);
      let value = files.get(target.file);
      for (const part of target.fragment.split('/').slice(1)) {
        value = value?.[part.replaceAll('~1', '/').replaceAll('~0', '~')];
        assert(value !== undefined, `unresolved reference ${file}: ${node.$ref}`);
      }
      assert(allowed[layers.get(file)]?.includes(layers.get(target.file)), `forbidden dependency ${file} -> ${target.file}`);
    });
  }
  const definitions = new Map();
  for (const [file, layer] of schemaLayers) {
    if (file === index.rpcEnvelopeSchema || file === provider.manifestSchemaFile) continue;
    const legacyLayer = file === provider.transportHandshakeSchemaFile ? 'common' : layer;
    for (const name of Object.keys(files.get(file).$defs ?? {})) {
      const key = `${legacyLayer}:${name}`;
      assert(!definitions.has(key), `duplicate type ${key}`);
      definitions.set(key, file);
    }
  }
  const diskFiles = (await readdir(root, { recursive: true })).filter(file => file.endsWith('.json')).map(file => file.split(path.sep).join('/'));
  assert(diskFiles.length === files.size && diskFiles.every(file => files.has(file)), 'unregistered JSON source file in protocol/v1');
  return { root, index, files, layers, schemaLayers, provider, gateway, control };
}

function rewrite(value, source, destination, mapping, { legacy = false } = {}) {
  if (Array.isArray(value)) return value.map(v => rewrite(v, source, destination, mapping, { legacy }));
  if (!value || typeof value !== 'object') return value;
  if (value.$ref) {
    const target = targetOf(value.$ref, source), file = mapping.get(target.file);
    assert(file, `cannot map ${source}: ${value.$ref}`);
    const relative = file === destination ? '' : path.posix.relative(path.posix.dirname(destination), file);
    const result = { $ref: `${relative}#${target.fragment}` };
    if (!legacy && value.description) result.description = value.description;
    return result;
  }
  if (legacy && value.type === 'null') return { type: 'null' };
  return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !legacy || ((!key.startsWith('x-codepet-') || key === 'x-codepet-sensitive') && key !== 'default'))
    .map(([key, child]) => [key, rewrite(child, source, destination, mapping, { legacy })]));
}
function mergedSchema(canonical, mapping, destination, { legacy = false } = {}) {
  const result = {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: `https://code-pet.dev/${legacy ? 'protocol/' : 'provider-sdk/v1/'}${destination}`,
    description: legacy ? 'Generated compatibility view of protocol/v1; edit the functional IDL modules and run protocol:generate.' : '由权威功能模块组装的协议类型。',
    $defs: {},
  };
  for (const [file, target] of mapping) if (target === destination) {
    const source = canonical.files.get(file);
    for (const [name, definition] of Object.entries(source.$defs ?? {})) {
      assert(!result.$defs[name], `duplicate type ${destination}: ${name}`);
      result.$defs[name] = rewrite(definition, file, destination, mapping, { legacy });
    }
    if (!legacy && source['x-codepet-lifecycle']) result['x-codepet-lifecycle'] = source['x-codepet-lifecycle'];
  }
  return result;
}
function registry(canonical, service, kind, destination, mapping, options) {
  return service.modules.flatMap(module => (module[kind]?.[kind] ?? []).map(item => rewrite(item, module[`${kind}File`], destination, mapping, options)));
}

export function providerPublicationInputs(canonical) {
  const { provider, index } = canonical;
  const sdk = provider.sdkManifest, mapping = new Map([
    [index.commonSchema, sdk.commonSchema], [index.agentDomainSchema, sdk.agentDomainSchema],
    [index.rpcEnvelopeSchema, sdk.rpcEnvelopeSchema], [provider.manifestSchemaFile, sdk.manifestSchema],
    [provider.profilesFile, sdk.profiles], [provider.transportHandshakeSchemaFile, sdk.transportHandshakeSchema],
  ]);
  for (const module of provider.modules) mapping.set(module.schemaFile, module.id === 'lifecycle' ? sdk.providerLifecycleSchema : sdk.providerMessagesSchema);
  const files = new Map([['sdk-manifest.json', structuredClone(sdk)]]);
  for (const key of ['commonSchema', 'agentDomainSchema', 'rpcEnvelopeSchema', 'manifestSchema', 'profiles', 'transportHandshakeSchema']) {
    const source = [...mapping].find(([, target]) => target === sdk[key])[0];
    const value = rewrite(canonical.files.get(source), source, sdk[key], mapping);
    if (value.$id) value.$id = `https://code-pet.dev/provider-sdk/v1/${sdk[key]}`;
    files.set(sdk[key], value);
  }
  for (const key of ['providerLifecycleSchema', 'providerMessagesSchema']) files.set(sdk[key], mergedSchema(canonical, mapping, sdk[key]));
  const methods = registry(canonical, provider, 'methods', sdk.methods, mapping);
  const events = registry(canonical, provider, 'events', sdk.events, mapping);
  files.set(sdk.methods, { description: '由 Provider 功能模块生成的方法注册表。', methods, service: rewrite(provider.entry.service, provider.file, sdk.methods, mapping) });
  files.set(sdk.events, { description: '由 Provider 功能模块生成的事件注册表。', events });
  return { files, index: sdk };
}

export function modularCompatibilityViews(canonical) {
  const { index, provider, gateway, control } = canonical;
  const paths = { common: 'core/v1/schema.json', agent: 'agent/v1/schema.json', provider: 'provider/v1/schema.json', gateway: 'gateway/v1/schema.json', pairing: 'channel/lan/v1/schema.json', control: 'channel/control/v1/schema.json' };
  const mapping = new Map();
  for (const [file, layer] of canonical.schemaLayers) {
    if (file === index.rpcEnvelopeSchema || file === provider.manifestSchemaFile) continue;
    mapping.set(file, file === provider.transportHandshakeSchemaFile ? paths.common : paths[layer]);
  }
  const views = new Map();
  for (const destination of Object.values(paths)) views.set(destination, encode(mergedSchema(canonical, mapping, destination, { legacy: true })));
  for (const service of [provider, gateway]) {
    const destination = service === provider ? 'provider/v1/manifest.json' : 'gateway/v1/manifest.json';
    const metadata = rewrite(service.entry.service, service.file, destination, mapping, { legacy: true });
    const manifest = { ...metadata, description: 'Generated compatibility view of protocol/v1; do not edit.', version: service.entry.version };
    if (service === provider) manifest.transport = {
      kind: 'json-rpc-2.0', framing: provider.profiles.defaultProfile,
      hello: { $ref: '../../core/v1/schema.json#/$defs/ProviderTransportHello' },
      selection: { $ref: '../../core/v1/schema.json#/$defs/ProviderTransportSelection' },
      requestDiscriminator: 'method', eventDiscriminator: 'method', jsonRpcVersion: canonical.files.get(index.rpcEnvelopeSchema).$defs.Request.properties.jsonrpc.const,
      requestId: { $ref: '../../core/v1/schema.json#/$defs/RequestId' }, rpcError: { $ref: '../../core/v1/schema.json#/$defs/RpcError' },
    };
    else Object.assign(manifest.transport, service.entry.compatibility);
    manifest.methods = registry(canonical, service, 'methods', destination, mapping, { legacy: true });
    manifest.events = registry(canonical, service, 'events', destination, mapping, { legacy: true });
    manifest.fixtures = 'fixtures/index.json';
    views.set(destination, encode(manifest));
  }
  const pairing = gateway.modules.find(module => module.id === 'pairing');
  views.set('channel/lan/v1/manifest.json', encode({ ...pairing.service.compatibilityService, description: 'Generated compatibility view of Gateway pairing; legacy package name retained.' }));
  views.set('channel/control/v1/manifest.json', encode({ ...control.entry.service, description: 'Generated compatibility view of protocol/v1/channel-control.' }));
  return views;
}
