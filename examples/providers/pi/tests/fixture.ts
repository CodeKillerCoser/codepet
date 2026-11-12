import { createServer } from 'node:http';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// A deterministic model endpoint. Pi itself, its SDK, tools and sessions are real.
// This verifies integration without spending tokens or requiring user credentials.
export async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'codepet-pi-'));
  const agent = join(root, 'agent'); await mkdir(agent);
  const workspace = join(root, 'workspace'); await mkdir(workspace);
  await writeFile(join(workspace, 'proof.txt'), 'PI_TOOL_PROOF');
  let modelRequests = 0; let toolResults = 0;
  const server = createServer(async (req, res) => {
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString()); modelRequests++;
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    const send = (delta: object, finish: string | null = null) => res.write(`data: ${JSON.stringify({ id: 'test', object: 'chat.completion.chunk', created: 1, model: 'pi-test', choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`);
    const text = JSON.stringify(body.messages);
    if (text.includes('WAIT_FOR_INTERRUPT')) {
      send({ role: 'assistant', content: 'waiting' });
      return; // Aborted by the client; no timers or automatic retry.
    }
    const tool = body.messages.findLast((m: { role: string }) => m.role === 'tool');
    if (!tool) {
      send({ role: 'assistant', tool_calls: [{ index: 0, id: 'read-proof', type: 'function', function: { name: 'read', arguments: JSON.stringify({ path: 'proof.txt' }) } }] });
      send({}, 'tool_calls');
    } else {
      if (JSON.stringify(tool).includes('PI_TOOL_PROOF')) toolResults++;
      send({ role: 'assistant', content: 'PI_VALIDATION_' }); send({ content: 'OK' }); send({}, 'stop');
    }
    res.end('data: [DONE]\n\n');
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = (server.address() as { port: number }).port;
  await writeFile(join(agent, 'models.json'), JSON.stringify({ providers: { 'codepet-validation': {
    baseUrl: `http://127.0.0.1:${port}/v1`, api: 'openai-completions', apiKey: 'test-only', models: [
      { id: 'pi-test', name: 'Pi integration fixture', reasoning: false, input: ['text'], contextWindow: 32768, maxTokens: 1024, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } },
    ],
  } } }));
  return { root, agent, workspace, counts: () => ({ modelRequests, toolResults }), close: async () => {
    server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve())); await rm(root, { recursive: true, force: true });
  } };
}
