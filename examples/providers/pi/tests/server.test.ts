import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough, Writable } from 'node:stream';
import { serve } from '../server.ts';

test('worker framing preserves split UTF-8 and U+2028, correlates concurrent requests and structured errors', async () => {
  const input = new PassThrough(); const chunks: Buffer[] = [];
  const output = new Writable({ write(chunk, _encoding, callback) { chunks.push(Buffer.from(chunk)); callback(); } });
  const server = await serve({ 'provider.describe': async () => {
    await new Promise((resolve) => setTimeout(resolve, 5));
    return { plugin: { pluginId: 'test', displayName: '你好\u2028pi', version: '1', instanceKinds: ['pi'], supportedVersions: { minVersion: 1, maxVersion: 1 } } };
  } }, input, output);
  const done = server.run();
  const bytes = Buffer.from([
    { jsonrpc: '2.0', id: 'one', method: 'provider.describe', params: { test: '你好\u2028pi' } },
    { jsonrpc: '2.0', id: 'two', method: 'unsupported', params: {} },
  ].map((v) => JSON.stringify(v)).join('\n') + '\n');
  for (let offset = 0; offset < bytes.length; offset += 3) input.write(bytes.subarray(offset, offset + 3));
  input.end(); await done;
  const records = Buffer.concat(chunks).toString().trim().split('\n').map((line) => JSON.parse(line));
  assert.equal(records.length, 2);
  assert.equal(records.find((v) => v.id === 'one').result.plugin.displayName, '你好\u2028pi');
  assert.equal(records.find((v) => v.id === 'two').error.data.code, 'method_not_implemented');
});

test('worker rejects truncated input instead of interpreting it as a complete request', async () => {
  const input = new PassThrough();
  const server = await serve({}, input, new PassThrough());
  const done = server.run(); input.end('{"jsonrpc":"2.0"}');
  await assert.rejects(done, /Truncated worker record/);
});
