import test from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { loadModularContract, modularCompatibilityViews, providerPublicationInputs } from './modular-contract.mjs';
const source = path.resolve(import.meta.dirname, '../../protocol');
async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codepet-modular-'));
  await cp(path.join(source, 'v2'), path.join(root, 'v2'), { recursive: true });
  await cp(path.join(source, 'codegen.json'), path.join(root, 'codegen.json'));
  return root;
}
async function edit(root, file, update) {
  const target = path.join(root, 'v2', file), value = JSON.parse(await readFile(target));
  update(value); await writeFile(target, JSON.stringify(value));
}
function wireShape(value) {
  if (Array.isArray(value)) return value.map(wireShape);
  if (!value || typeof value !== 'object') return value;
  const result = Object.fromEntries(Object.keys(value).sort().filter(k => !['description', '$id', 'title'].includes(k)).map(k => [k, k === '$ref' ? value[k].replace(/^\.\//, '') : wireShape(value[k])]));
  for (const key of ['methods', 'events']) if (Array.isArray(result[key])) result[key].sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  return result;
}
test('modular migration preserves the pre-migration wire surface and all schema constraints', async () => {
  const expected = JSON.parse(await readFile(new URL('./fixtures/v1-wire-shapes.json', import.meta.url)));
  const views = modularCompatibilityViews(await loadModularContract(source, {version: 1}));
  for (const [file, hash] of Object.entries(expected)) {
    const value = JSON.parse(views.get(file) ?? await readFile(path.join(source, file)));
    assert.equal(createHash('sha256').update(JSON.stringify(wireShape(value))).digest('hex'), hash, file);
  }
});
test('shared type changes reach both SDK publication and compatibility consumers without old source files', async () => {
  const root = await fixture();
  await edit(root, 'common/types.schema.json', d => d.$defs.TimestampMs.description = 'Shared timestamp defined once');
  const canonical = await loadModularContract(root), published = providerPublicationInputs(canonical);
  assert.equal(published.files.get(published.index.commonSchema).$defs.TimestampMs.description, 'Shared timestamp defined once');
  assert.equal(JSON.parse(modularCompatibilityViews(canonical).get('core/v2/schema.json')).$defs.TimestampMs.description, 'Shared timestamp defined once');
  assert.ok(canonical.gateway.modules.every(m => m.id === 'pairing' || m.schemaFile.startsWith('gateway/')));
  assert.ok(!canonical.gateway.entry.service.transport.framing, 'business protocol is transport-neutral');
});
test('a Gateway method is owned by one functional registry and aggregation follows that registry', async () => {
  const root = await fixture();
  await edit(root, 'gateway/project/methods.json', d => d.methods.find(m => m.name === 'project.get').name = 'project.read');
  const canonical = await loadModularContract(root), views = modularCompatibilityViews(canonical);
  const names = JSON.parse(views.get('gateway/v2/manifest.json')).methods.map(m => m.name);
  assert.ok(names.includes('project.read')); assert.ok(!names.includes('project.get'));
  assert.equal(canonical.gateway.entry.methods, undefined);
  assert.equal(canonical.gateway.entry.service.methods, undefined);
});
test('module graph rejects duplicate methods/events/types, missing descriptions, dangling refs and upward dependencies', async () => {
  const cases = [
    ['gateway/project/methods.json', d => d.methods.push(structuredClone(d.methods[0])), /duplicate methods/],
    ['gateway/project/events.json', d => d.events.push(structuredClone(d.events[0])), /duplicate events/],
    ['gateway/turn/messages.schema.json', d => d.$defs.ProviderIdentity = { type: 'object', description: 'Conflicting type' }, /duplicate type/],
    ['gateway/project/messages.schema.json', d => delete d.$defs.ProjectGetRequest.properties.project.description, /missing description/],
    ['gateway/project/messages.schema.json', d => d.$defs.ProjectGetRequest.properties.project.$ref = './missing.schema.json#/$defs/Project', /missing reference/],
    ['gateway/project/messages.schema.json', d => d.$defs.ProjectGetRequest.properties.project.$ref = '../../provider/project/messages.schema.json#/$defs/ProjectGetRequest', /forbidden dependency/],
    ['common/types.schema.json', d => d.$defs.RequestId.$ref = '../gateway/session/messages.schema.json#/$defs/PingRequest', /forbidden dependency/],
  ];
  for (const [file, update, error] of cases) {
    const root = await fixture(); await edit(root, file, update); await assert.rejects(loadModularContract(root), error);
  }
});
test('pairing belongs to Gateway while its legacy transport binding and credential sensitivity stay intact', async () => {
  const canonical = await loadModularContract(source), module = canonical.gateway.modules.find(m => m.id === 'pairing');
  assert.equal(module.methods, undefined, 'pre-auth HTTP operations must not become authenticated business RPC');
  assert.ok(module.bindings.operations.some(v => v.name === 'pairing.exchange'));
  const schema = JSON.parse(modularCompatibilityViews(canonical).get('channel/lan/v1/schema.json'));
  assert.equal(schema.$defs.PairingExchangeResponse.properties.credential['x-codepet-sensitive'], true);
  const publication = providerPublicationInputs(canonical);
  assert.ok([...publication.files.keys()].every(file => !/(gateway|pairing|channel)/.test(file)));
});

test('v2 is active and every schema has descriptions and closed references', async () => {
    const live = await loadModularContract(source);
    assert.equal(live.index.version, 2);
    assert.equal(live.index.minVersion, 2);
    assert.deepEqual(live.index.compatibility.offerFields, ['version', 'minVersion']);
});

test('active negotiation distinguishes software version, offer and selected connection version', async () => {
  const read = async file => JSON.parse(await readFile(path.join(source, 'v2', file)));
  const version = (await read('common/types.schema.json')).$defs;
  for (const type of ['ProtocolVersionOffer', 'ProtocolVersionSelection']) {
    assert.equal(version[type].additionalProperties, true);
    assert.ok(version[type].required.includes('version'));
    assert.ok(version[type].required.includes('minVersion'));
    assert.ok(!Object.hasOwn(version[type].properties, 'maxVersion'));
  }
  assert.ok(version.ProtocolVersionSelection.required.includes('selectedVersion'));
  for (const [file, request, response] of [
    ['provider/lifecycle/messages.schema.json', 'ProviderInitializeRequest', 'ProviderInitializeResponse'],
    ['gateway/session/messages.schema.json', 'HandshakeRequest', 'HandshakeResponse'],
  ]) {
    const definitions = (await read(file)).$defs;
    assert.match(definitions[request].properties.protocol.$ref, /ProtocolVersionOffer$/);
    assert.match(definitions[response].properties.protocol.$ref, /ProtocolVersionSelection$/);
    for (const name of [request, response]) {
      assert.equal(definitions[name].additionalProperties, true);
      assert.ok(definitions[name].required.includes('protocol'));
      assert.ok(!Object.hasOwn(definitions[name].properties, 'supportedVersions'));
      assert.ok(!Object.hasOwn(definitions[name].properties, 'selectedVersion'));
    }
  }
  const plugin = (await read('provider/lifecycle/messages.schema.json')).$defs.ProviderPluginDescriptor;
  assert.equal(plugin.properties.version.type, 'string');
  assert.match(plugin.properties.protocol.$ref, /ProtocolVersionOffer$/);
  const policy = (await read('protocol.json')).compatibility;
  assert.equal(policy.selectionFormula, 'min(local.version, peer.version)');
  assert.equal(policy.acceptanceFormula, 'selectedVersion >= max(local.minVersion, peer.minVersion)');
  assert.equal(policy.malformedKnownValues, 'reject; never-coerce-or-treat-as-unknown');
  assert.equal(policy.writeRetries, 'never-replay-unknown-outcomes');
});

test('active negotiation examples cover boundaries, disjoint ranges and invalid offers', async () => {
  const draft = JSON.parse(await readFile(path.join(source, 'v2/protocol.json')));
  assert.ok(draft.negotiationExamples.length >= 5);
  const valid = value => Number.isInteger(value.version) && Number.isInteger(value.minVersion)
    && value.minVersion >= 1 && value.version <= 4294967295 && value.minVersion <= value.version;
  for (const example of draft.negotiationExamples) {
    const selected = Math.min(example.local.version, example.peer.version);
    const compatible = valid(example.local) && valid(example.peer)
      && selected >= Math.max(example.local.minVersion, example.peer.minVersion);
    assert.equal(compatible, example.compatible, example.description);
    assert.equal(compatible ? selected : null, example.selectedVersion, example.description);
  }
});

test('Provider and Gateway revisions evolve independently inside the source directory', async () => {
  const root=await fixture();
  await edit(root,'provider/protocol.json',d=>{d.version=3;d.minVersion=2;});
  await edit(root,'provider/sdk-manifest.json',d=>{d.protocolVersion=3;});
  await edit(root,'gateway/protocol.json',d=>{d.version=4;d.minVersion=2;});
  const canonical=await loadModularContract(root), views=modularCompatibilityViews(canonical);
  assert.equal(canonical.index.version,2);
  assert.equal(canonical.provider.sdkManifest.sdkVersion,2);
  assert.equal(JSON.parse(views.get('provider/v2/manifest.json')).version,3);
  assert.equal(JSON.parse(views.get('gateway/v2/manifest.json')).version,4);
  assert.equal(JSON.parse(views.get('gateway/v2/manifest.json')).minVersion,2);
});
