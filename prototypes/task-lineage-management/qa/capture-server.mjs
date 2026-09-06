import http from 'node:http';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const names = new Set(['implementation-conversation.png', 'implementation-graph.png', 'implementation-swimlane.png']);
const server = http.createServer((request, response) => {
  const name = request.url?.slice(1);
  if (request.method !== 'POST' || !names.has(name)) {
    response.writeHead(404).end();
    return;
  }
  const chunks = [];
  request.on('data', chunk => chunks.push(chunk));
  request.on('end', async () => {
    await writeFile(join(import.meta.dirname, name), Buffer.concat(chunks));
    response.writeHead(204, { 'Access-Control-Allow-Origin': '*' }).end();
  });
});

server.listen(4180, '127.0.0.1');
