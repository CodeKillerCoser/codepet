import test from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { loadProviderContract, publishProviderContract } from './provider-contract.mjs';
import { modularCompatibilityViews } from './modular-contract.mjs';
import { providerRuntimeArtifacts } from './provider-runtime.mjs';
const source = path.resolve(import.meta.dirname, '../../protocol');
async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codepet-idl-'));
  await cp(path.join(source, 'v1'), path.join(root, 'v1'), { recursive: true });
  return root;
}
async function change(root, file, edit) {
  const p = path.join(root, 'v1', file), value = JSON.parse(await readFile(p));
  edit(value); await writeFile(p, JSON.stringify(value));
}

test('one IDL change reaches runtime constants, compatibility DTOs, registry envelopes and published documentation', async () => {
  const root = await fixture();
  await change(root, 'provider/lifecycle/messages.schema.json', d => { d['x-codepet-lifecycle'].heartbeatIntervalMs = 15000; d.$defs.ProviderPingResponse.properties.sequence.description = 'IDL-owned reply sequence'; });
  await change(root, 'provider/transport/handshake.schema.json', d => d.$defs.ProviderTransportLimits.properties.normalStreams.default = 20);
  await change(root, 'provider/lifecycle/methods.json', d => d.methods.find(m => m.name === 'provider.describe').name = 'provider.identity');
  const model = await loadProviderContract(root), published = publishProviderContract(model);
  const runtime = providerRuntimeArtifacts(model, published), views = modularCompatibilityViews(model.canonical);
  assert.match(runtime.get('sdk/rust/codepet-provider-sdk/src/runtime/protocol.rs'), /from_millis\(15000\)/);
  assert.match(runtime.get('sdk/rust/codepet-transport/src/protocol.rs'), /normal_streams: 20/);
  assert.match(published.get('README.md'), /等待 15 秒/);
  assert.equal(JSON.parse(views.get('provider/v1/schema.json')).$defs.ProviderPingResponse.properties.sequence.description, 'IDL-owned reply sequence');
  assert.ok(JSON.parse(views.get('provider/v1/manifest.json')).methods.some(m => m.name === 'provider.identity'));
  const envelope = JSON.parse(published.get(model.index.rpcEnvelopeSchema));
  assert.ok(envelope.$defs.Request.oneOf.some(v => v.properties.method.const === 'provider.identity'));
  assert.ok(!envelope.$defs.Request.oneOf.some(v => v.properties.method.const === 'provider.describe'));
});
test('missing descriptions, unresolved refs, invalid defaults and contradictory envelope shapes fail before publishing', async () => {
  for (const [file, edit, expected] of [
    ['provider/lifecycle/messages.schema.json', d => delete d.$defs.ProviderPingRequest.properties.sequence.description, /description/],
    ['provider/turn/messages.schema.json', d => d.$defs.TurnStartRequest.properties.input.$ref = './missing.json#/$defs/TurnInput', /missing reference/],
    ['provider/transport/handshake.schema.json', d => d.$defs.ProviderTransportLimits.properties.normalStreams.default = 99, /invalid default/],
    ['common/rpc.schema.json', d => delete d.$defs.Request.properties.id, /envelope shape/],
  ]) {
    const root = await fixture(); await change(root, file, edit); await assert.rejects(loadProviderContract(root), expected);
  }
});
test('manifest transport values come from the same profile registry; generated outputs are deterministic', async () => {
  const model = await loadProviderContract(source), a = publishProviderContract(model), b = publishProviderContract(model);
  assert.deepEqual([...a], [...b]);
  assert.deepEqual(JSON.parse(a.get(model.index.manifestSchema)).properties.transport.enum, model.profiles.map(p => p.id));
  assert.equal(model.files.get(model.index.manifestSchema).properties.transport.enum, undefined);
  assert.ok(!model.files.get(model.index.rpcEnvelopeSchema).$defs.Request.oneOf);
});
test('installation code generation rejects unsupported validation keywords instead of ignoring constraints', async () => {
  const root = await fixture(); await change(root, 'provider/manifest.schema.json', d => d.properties.command.format = 'uri');
  const model = await loadProviderContract(root);
  assert.throws(() => providerRuntimeArtifacts(model, publishProviderContract(model)), /Unsupported installation Schema keyword format/);
});
