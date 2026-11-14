import test from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
import { loadProtocolModel, generatorTargetRegistry } from './generate.mjs';

test('generated TypeScript validates known data and preserves only future receive values', async () => {
  const model=await loadProtocolModel();
  const record=model.recordsById.get('gateway-v1');
  const source=generatorTargetRegistry.typescript.render({record,model,ir:model.protocolIr});
  const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;
  const sdk=await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
  const response={protocol:{version:4,minVersion:2,selectedVersion:2},device:{name:'Host',operatingSystem:'Test',systemVersion:'1'},providers:[],eventCursor:'e-1',future:true};
  assert.deepEqual(sdk.decodeProtocolValue('gateway:HandshakeResponse',response),response);
  for(const field of ['minVersion','selectedVersion']) {
    const malformed=structuredClone(response);malformed.protocol[field]='2';
    assert.throws(()=>sdk.decodeProtocolValue('gateway:HandshakeResponse',malformed));
  }
  const unknown={kind:'future-kind',vendor:{raw:7}};
  assert.equal(sdk.decodeProtocolValue('agent:ConversationItem',unknown),unknown);
  assert.equal(sdk.decodeProtocolValue('agent:ConversationStatus','future-status'),'future-status');
  assert.throws(()=>sdk.encodeProtocolValue('agent:ConversationItem',unknown));
  assert.throws(()=>sdk.encodeProtocolValue('agent:ApprovalDecision','approve-with-new-policy'));
  assert.throws(()=>sdk.decodeProtocolValue('agent:ConversationItem',{kind:'message',status:'completed'}));
  assert.throws(()=>sdk.decodeProtocolValue('gateway:HandshakeResponse',{...response,eventCursor:''}));
});

test('TypeScript common range helpers validate highest selection and reject invalid offers',async()=>{
  const model=await loadProtocolModel(), record=model.recordsById.get('core-v1');
  const source=generatorTargetRegistry.typescript.render({record,model,ir:model.protocolIr});
  const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;
  const sdk=await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
  const local={version:4,minVersion:2},peer={version:5,minVersion:3};
  const response=sdk.negotiateProtocolVersion(peer,local);
  assert.deepEqual(response,{version:5,minVersion:3,selectedVersion:4});
  sdk.validateProtocolSelection(local,response);
  assert.throws(()=>sdk.validateProtocolSelection(local,{...response,selectedVersion:3}));
  for(const bad of [{version:0,minVersion:0},{version:3,minVersion:4},{version:1,minVersion:1},{version:4294967296,minVersion:2}]) assert.throws(()=>sdk.negotiateProtocolVersion(local,bad));
});

test('schema resolution uses native keys for Windows drives, Unicode and URI escapes', async () => {
  const { win32, posix } = await import('node:path');
  const { resolveSchemaPath, legacySchemaPath } = await import('./schema-paths.mjs');
  for (const paths of [win32, posix]) {
    const root = paths === win32 ? 'C:\\Users\\协议 空间' : '/Users/协议 空间';
    const schema = paths.join(root, 'protocol', 'gateway', 'v2', 'schema.json');
    assert.equal(resolveSchemaPath(schema, '../../../core/v2/schema.json', paths), paths.join(root, 'core', 'v2', 'schema.json'));
    assert.equal(resolveSchemaPath(schema, '../v2/a%20b.json', paths), paths.join(paths.dirname(schema), 'a b.json'));
    assert.equal(legacySchemaPath(schema, 2, paths), paths.join(root, 'protocol', 'gateway', 'v1', 'schema.json'));
    assert.throws(() => legacySchemaPath(paths.join(root, 'channel', 'v1', 'schema.json'), 2, paths));
  }
});

test('legacy DTO refs and validation graph resolve from a checkout containing spaces', async () => {
  const { compatibilityGraph } = await import('./compatibility.mjs');
  const { resolveSchemaPath } = await import('./schema-paths.mjs');
  const { configureProtocolGenerator, repositoryRoot } = await import('./generate.mjs');
  const { cp, mkdtemp, rm } = await import('node:fs/promises');
  const path = await import('node:path');
  const { tmpdir } = await import('node:os');
  const root = await mkdtemp(path.join(tmpdir(), 'codepet 协议 空间-'));
  try {
    await cp(path.join(repositoryRoot, 'protocol'), path.join(root, 'protocol'), { recursive: true });
    configureProtocolGenerator({ sourceRoot: root });
    const model = await loadProtocolModel();
    const core = model.recordsById.get('core-v1');
    const pet = model.recordsById.get('pet-v1');
    assert.equal(model.schemasByPath.get(resolveSchemaPath(pet.schemaPath, '../../core/v1/schema.json')), core);
    const graph = compatibilityGraph(model.recordsById.get('gateway-v1'), model);
    assert.ok(graph.definitions['core:ProtocolVersionOffer']);
  } finally {
    configureProtocolGenerator({ sourceRoot: repositoryRoot });
    await rm(root, { recursive: true, force: true });
  }
});
