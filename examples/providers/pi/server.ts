import { once } from 'node:events';
import type { Readable, Writable } from 'node:stream';
import type { ProtocolRequestMap, ProtocolResponseMap, ProtocolEvent } from './types/codepet-provider-sdk/src/generated.ts';

export type Handlers = { [M in keyof ProtocolRequestMap]?: (params: ProtocolRequestMap[M]) => Promise<ProtocolResponseMap[M]> | ProtocolResponseMap[M] };
export class ProviderError extends Error {
  code: string;
  retryable: boolean;
  constructor(code: string, message: string, retryable = false) { super(message); this.code = code; this.retryable = retryable; }
}
// Internal language bridge, not an alternative Host transport. LF only: JSON may contain U+2028.
export async function serve(handlers: Handlers, input: Readable = process.stdin, output: Writable = process.stdout) {
  let pending = Buffer.alloc(0);
  const active = new Set<Promise<void>>();
  let writes = Promise.resolve();
  function write(record: object) {
    const line = JSON.stringify(record) + '\n';
    if (Buffer.byteLength(line) > 16 * 1024 * 1024) throw new Error('Worker record limit exceeded');
    writes = writes.then(async () => { if (!output.write(line)) await once(output, 'drain'); });
    return writes;
  }
  const publish = (event: ProtocolEvent) => write(event);
  async function dispatch(line: Buffer) {
    const request = JSON.parse(line.toString('utf8'));
    if (request.jsonrpc !== '2.0' || typeof request.id !== 'string') throw new Error('Invalid worker request');
    try {
      const handler = handlers[request.method as keyof Handlers];
      if (!handler) throw new ProviderError('method_not_implemented', `Unsupported method: ${request.method}`);
      const result = await (handler as (params: unknown) => unknown)(request.params);
      await write({ jsonrpc: '2.0', id: request.id, result });
    } catch (cause) {
      const error = cause instanceof ProviderError ? cause : new ProviderError('pi_provider_failed', String(cause));
      await write({ jsonrpc: '2.0', id: request.id, error: { code: -32000, message: error.message,
        data: { code: error.code, message: error.message, retryable: error.retryable } } });
    }
  }
  async function run() {
    for await (const chunk of input) {
      pending = Buffer.concat([pending, Buffer.from(chunk)]);
      for (;;) {
        const end = pending.indexOf(10);
        if (end < 0) break;
        if (end > 16 * 1024 * 1024) throw new Error('Worker record limit exceeded');
        const line = pending.subarray(0, end);
        pending = pending.subarray(end + 1);
        if (active.size >= 64) await Promise.race(active);
        const task = dispatch(line);
        active.add(task);
        void task.finally(() => active.delete(task)).catch(() => {});
      }
      if (pending.length > 16 * 1024 * 1024) throw new Error('Worker record limit exceeded');
    }
    if (pending.length) throw new Error('Truncated worker record');
    await Promise.all(active);
    await writes;
  }
  return { publish, run };
}
