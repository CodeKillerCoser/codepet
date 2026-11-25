import { copyFile, mkdir, readFile, writeFile, readdir } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { extractSnapshot } from '../../cognee-task-recall/native/cognee-renderer/snapshot.mjs';

const source=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/ladybug-cognee-renderer');
const target=resolve(process.argv[3]||'artifacts/cognee-task-recall-qa/cognee-graph-view');
const here=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const sha=value=>createHash('sha256').update(value).digest('hex');
const provenance=JSON.parse(await readFile(join(source,'provenance.json'),'utf8'));
for(const dir of ['src','assets','licenses','example']) await mkdir(join(target,dir),{recursive:true});
for(const file of ['index.mjs','assemble.mjs','frame.mjs']) await copyFile(join(here,'src',file),join(target,'src',file));
for(const file of ['frame.html','package.json','README.md']) await copyFile(join(here,file),join(target,file));
await copyFile(join(here,'example/index.html'),join(target,'example/index.html'));
for(const asset of provenance.upstreamAssets) {
  const file=asset.source.split('/').at(-1),data=await readFile(join(source,'assets',file));
  if(sha(data)!==asset.sha256) throw new Error(`Upstream asset hash mismatch: ${file}`);
}
let frontendBytes=0;
for(const file of await readdir(join(source,'assets'))) {
  if(file==='inter-upstream.css')continue;
  let data=await readFile(join(source,'assets',file));
  if(file==='inter.css')data=Buffer.from(data.toString().replaceAll('url(/assets/','url(./'));
  await writeFile(join(target,'assets',file),data);frontendBytes+=data.length;
}
for(const file of await readdir(join(source,'licenses'))) await copyFile(join(source,'licenses',file),join(target,'licenses',file));
// Demo-only input: reuse the prior native read-back export, without opening any database.
const snapshot=extractSnapshot(await readFile(join(source,'index.html'),'utf8'));
await writeFile(join(target,'example/graph.json'),JSON.stringify(snapshot));
await writeFile(join(target,'provenance.json'),JSON.stringify({createdAt:new Date().toISOString(),
  upstream:{version:provenance.cogneeVersion,assets:provenance.upstreamAssets},frontendBytes,
  runtime:{browserOnly:true,databaseRequired:false,pythonRequired:false,nodeBackendRequired:false},
  demo:{source:join(source,'index.html'),nodes:snapshot.nodes.length,links:snapshot.links.length}},null,2));
console.log(JSON.stringify({target,frontendBytes,nodes:snapshot.nodes.length,links:snapshot.links.length}));
