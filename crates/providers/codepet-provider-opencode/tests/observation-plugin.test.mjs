import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';

async function fixture(t, get = async () => ({ data: { title: 'Existing task', directory: '/work', parentID: 'parent' } })) {
  const root = await mkdtemp(join(tmpdir(), 'codepet-stable-plugin-'));
  await mkdir(join(root, 'plugins'));
  await mkdir(join(root, 'codepet-observation'));
  const pluginPath = join(root, 'plugins', 'observation.mjs');
  await writeFile(pluginPath, await readFile(new URL('../src/observation-plugin.ts', import.meta.url)));
  const received = [];
  let status = 204;
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    received.push({ authorization: req.headers.authorization, ...JSON.parse(Buffer.concat(chunks).toString()) });
    res.writeHead(status).end();
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  await writeFile(join(root, 'codepet-observation', 'endpoint.json'), JSON.stringify({
    url: `http://127.0.0.1:${server.address().port}/event`, token: 'private-token',
  }));
  const { CodePetObservation } = await import(pathToFileURL(pluginPath).href);
  const hooks = await CodePetObservation({ client: { session: { get } }, directory: '/fallback' });
  t.after(async () => { await hooks.dispose(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); await rm(root, { recursive: true, force: true }); });
  const waitFor = async (count) => {
    for (let i = 0; received.length < count && i < 200; i++) await delay(10);
    assert.equal(received.length, count);
    return received[count - 1];
  };
  return { hooks, received, waitFor, reject: () => { status = 503; }, accept: () => { status = 204; } };
}
const event = (id, type, properties = {}) => ({ event: { id, type, properties: { sessionID: 'session', ...properties } } });

test('stable hooks preserve native IDs and metadata, filter noise, and stop on dispose', async t => {
  let lookups = 0;
  const f = await fixture(t, async () => { lookups++; throw Error('cached metadata should be used'); });
  await f.hooks.event(event('noise', 'message.part.updated'));
  await f.hooks.event(event('created', 'session.created', { info: { id: 'session', title: 'Stable task', directory: '/project' } }));
  await f.hooks.event(event('busy', 'session.status', { status: { type: 'busy' } }));
  await f.hooks.event(event('permission', 'permission.asked'));
  await f.hooks.event(event('question', 'question.asked'));
  await f.hooks.event(event('idle', 'session.idle'));
  await f.waitFor(5);
  assert.deepEqual(f.received.map(e => e.eventId), ['created', 'busy', 'permission', 'question', 'idle']);
  assert.equal(f.received[1].payload.session.title, 'Stable task');
  assert.equal(f.received[1].payload.cwd, '/project');
  assert.equal(f.received[1].payload.properties.status.type, 'busy');
  assert.equal(f.received[1].authorization, 'private-token');
  assert.equal(lookups, 0);
  await f.hooks.dispose();
  await f.hooks.event(event('after-dispose', 'session.idle'));
  await delay(30);
  assert.equal(f.received.length, 5);
});

test('existing sessions use the stable SDK response and failures mark a delivery gap', async t => {
  const calls = [];
  const f = await fixture(t, async options => {
    calls.push(options);
    return { data: { title: 'Existing task', directory: '/work', parentID: 'parent' } };
  });
  f.reject();
  await f.hooks.event(event('busy', 'session.status', { status: { type: 'busy' } }));
  const first = await f.waitFor(1);
  assert.equal(first.payload.session.parentID, 'parent');
  assert.equal(first.payload.cwd, '/work');
  assert.deepEqual(calls[0].path, { id: 'session' });
  assert.ok(calls[0].signal instanceof AbortSignal);
  await delay(30);
  f.accept();
  await f.hooks.event(event('idle', 'session.idle'));
  assert.equal((await f.waitFor(2)).payload.codepet_gap, true);
});

test('slow metadata lookup does not block hooks and dispose aborts it', async t => {
  let started;
  const pending = new Promise(resolve => { started = resolve; });
  const f = await fixture(t, ({ signal }) => new Promise((resolve, reject) => {
    started();
    signal.addEventListener('abort', () => reject(signal.reason), { once: true });
  }));
  await f.hooks.event(event('busy', 'session.status', { status: { type: 'busy' } }));
  await pending;
  await f.hooks.dispose();
  await delay(30);
  assert.equal(f.received.length, 0);
});
