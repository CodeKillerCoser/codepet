import { readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
// No stdout decisions: observation never approves, blocks, or continues a harness.
const timer = setTimeout(() => process.exit(0), 1500);
try {
  let input = '';
  for await (const chunk of process.stdin) {
    input += chunk;
    if (Buffer.byteLength(input) > 250000) process.exit(0);
  }
  const payload = JSON.parse(input);
  payload.codepet_observed_at = Date.now();
  const endpoint = JSON.parse(await readFile(process.argv[2], 'utf8'));
  await fetch(endpoint.url, { method: 'POST', headers: { 'content-type': 'application/json', authorization: endpoint.token },
    body: JSON.stringify({ eventId: randomUUID(), payload }), signal: AbortSignal.timeout(1000) });
} catch { /* Observations are best effort; never break the calling harness. */ }
clearTimeout(timer);
