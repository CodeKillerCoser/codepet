import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { createProvider } from '../provider.ts';
import { fixture } from './fixture.ts';
import type { ProtocolEvent, ProtocolRequestMap, ProtocolResponseMap } from '../types/codepet-provider-sdk/src/generated.ts';

test('real pi SDK: initialize, private SQLite, tool loop, deltas, interrupt and reload history', { timeout: 45000 }, async () => {
  const f = await fixture(); const before = process.env.CODEPET_PI_AGENT_DIR; process.env.CODEPET_PI_AGENT_DIR = f.agent;
  const events: ProtocolEvent[] = [];
  const provider = createProvider(async (event) => { events.push(structuredClone(event)); });
  const call = async <M extends keyof ProtocolRequestMap>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]> => {
    const handler = provider[method]! as (params: ProtocolRequestMap[M]) => ProtocolResponseMap[M];
    return handler(params);
  };
  const route = { deviceId: 'test-device', providerPluginId: 'dev.codepet.example.pi', providerInstanceId: 'pi-test-instance' };
  const initialize = { hostDeviceId: route.deviceId, hostClientId: 'host-test', hostVersion: '1', supportedVersions: { minVersion: 1, maxVersion: 1 },
    directories: { data: join(f.root, 'data'), logs: join(f.root, 'logs'), databasePath: join(f.root, 'data/usage.sqlite') } };
  async function waitFor(predicate: () => boolean) {
    const until = Date.now() + 20000;
    while (!predicate()) { assert.ok(Date.now() < until, 'event deadline exceeded'); await new Promise((resolve) => setTimeout(resolve, 10)); }
  }
  try {
    await call('provider.initialize', initialize);
    await call('instance.create', { route, instanceKind: 'pi', displayName: 'Pi test', settings: {} });
    await call('instance.start', { route });
    const created = await call('conversation.create', { route, permissionLevel: 'read-only', workspaceRoot: f.workspace });
    const conversation = { ...route, nativeResourceId: created.conversation.resource.nativeResourceId };
    const request = { conversation, clientRequestId: 'once', capabilityRevision: 'pi-readonly-v1', input: { kind: 'text' as const, text: 'Read proof.txt' },
      selection: { accessModeId: 'read-only', model: { kind: 'flat' as const, modelId: 'codepet-validation/pi-test' } } };
    const sent = await call('turn.start', request);
    assert.equal(sent.accepted, true);
    await waitFor(() => events.some((e) => e.method === 'event.turnUpserted' && e.params.turn.status === 'completed'));
    assert.ok(f.counts().toolResults > 0, 'pi must execute its real read tool and send its result to the model');
    const deltas = events.filter((e) => e.method === 'event.turnOutputDelta');
    assert.equal(deltas.map((e) => e.method === 'event.turnOutputDelta' ? e.params.delta : '').join(''), 'PI_VALIDATION_OK');
    const history = await call('conversation.get', { conversation });
    assert.equal(history.items.length, 3);
    assert.ok(history.items.some((item) => item.kind === 'tool' && item.status === 'completed'));
    assert.equal(history.items.find((item) => item.kind === 'message' && item.role === 'assistant')?.resource.nativeResourceId, deltas[0].method === 'event.turnOutputDelta' ? deltas[0].params.itemId : '');
    assert.equal((await call('turn.start', request)).turn.resource.nativeResourceId, sent.turn.resource.nativeResourceId);
    await assert.rejects(call('turn.start', { ...request, input: { kind: 'text', text: 'changed input' } }), /Request ID already used/);
    const slow = await call('turn.start', { ...request, clientRequestId: 'slow', input: { kind: 'text', text: 'WAIT_FOR_INTERRUPT' } });
    await waitFor(() => events.some((e) => e.method === 'event.turnOutputDelta' && e.params.delta === 'waiting'));
    assert.equal((await call('turn.interrupt', { conversation, turn: { ...route, nativeResourceId: slow.turn.resource.nativeResourceId } })).turn.status, 'interrupted');
    await assert.rejects(call('conversation.get', { conversation: { ...conversation, providerInstanceId: 'other' } }), /Unknown pi instance/);
    await call('provider.shutdown', {});
    const reloaded = createProvider(async () => {});
    await reloaded['provider.initialize']!(initialize);
    await reloaded['instance.create']!({ route, instanceKind: 'pi', displayName: 'Pi test', settings: {} });
    const saved = await reloaded['conversation.get']!({ conversation, limit: 1 });
    assert.equal(saved.items.length, 2); assert.ok(saved.pageInfo?.nextCursor);
    assert.equal(saved.items[1].status, 'interrupted');
    await reloaded['provider.shutdown']!({});
  } finally {
    if (before === undefined) delete process.env.CODEPET_PI_AGENT_DIR; else process.env.CODEPET_PI_AGENT_DIR = before;
    await f.close();
  }
});
