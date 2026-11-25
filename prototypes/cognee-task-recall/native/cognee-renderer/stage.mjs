import { readFile, writeFile, copyFile, mkdir, symlink, stat } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { GraphStore } from './store.mjs';
import { extractSnapshot, validateSnapshot, sha256 } from './snapshot.mjs';
import { render } from './render.mjs';
import { stageFonts } from './fonts.mjs';

const here=dirname(fileURLToPath(import.meta.url));
const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/ladybug-cognee-renderer');
try {await stat(join(root,'graph.lbdb'));throw new Error('Existing experiment database; use a new output directory');}catch(error){if(error.code!=='ENOENT')throw error;}
const installed=resolve('artifacts/cognee-task-recall-qa/.venv/lib/python3.12/site-packages');
const viz=join(installed,'cognee/modules/visualization');
const native=resolve('artifacts/cognee-task-recall-qa/native-comparison');
const previous=resolve('artifacts/cognee-task-recall-qa/runs/20261124T195156701196Z/cognee-graph.html');
const assets=join(root,'assets');
for(const dir of [assets,join(root,'app'),join(root,'native/vendor'),join(root,'licenses')]) await mkdir(dir,{recursive:true});
const manifest={createdAt:new Date().toISOString(),cogneeVersion:'1.6.10',license:'Apache-2.0',
  input:{path:previous,sha256:sha256(await readFile(previous))},upstreamAssets:[],
  notes:['Original Cognee JS and template copied byte-for-byte; D3 and font URLs localized during assembly.',
    'Previously processed graph, schema/memory metadata and semantic coordinates imported into native Ladybug.',
    'This prototype does not port Cognee extraction or recompute topology-dependent presentation metadata.'],
  runtime:{pythonRequired:false,nodeRequired:true,nativeLibrary:'Ladybug 0.22.3',modelCalls:0}};
for(const name of ['template.html',...['ui_chrome','schema_view','story_view','inspector','memory_map','semantic_map'].map(n=>'views/'+n+'.js')]) {
  const source=join(viz,name),target=join(assets,name.split('/').at(-1));
  await copyFile(source,target);
  const data=await readFile(target);
  manifest.upstreamAssets.push({source:name,bytes:data.length,sha256:sha256(data)});
}
for(const name of ['LICENSE','NOTICE.md']) await copyFile(join(installed,'cognee-1.6.10.dist-info/licenses',name),join(root,'licenses','cognee-'+name));
await copyFile(join(native,'ladybug-driver'),join(root,'native/ladybug-driver'));
await copyFile(join(native,'vendor/liblbug.0.22.3.dylib'),join(root,'native/vendor/liblbug.0.22.3.dylib'));
for(const name of ['liblbug.0.dylib','liblbug.dylib']) {
  try {await symlink('liblbug.0.22.3.dylib',join(root,'native/vendor',name));}catch(error){if(error.code!=='EEXIST')throw error;}
}
for(const name of ['store.mjs','render.mjs','snapshot.mjs','server.mjs']) await copyFile(join(here,name),join(root,'app',name));
await copyFile(join(here,'../ipc.mjs'),join(root,'ipc.mjs'));
const d3=join(assets,'d3.v7.9.0.min.js');
try {await stat(d3);}catch {
  await promisify(execFile)('curl',['-fsSL','--retry','2','--connect-timeout','15','--max-time','60',
    'https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js','-o',d3]);
}
const d3License=join(root,'licenses/d3-LICENSE');
try {await stat(d3License);}catch {
  await promisify(execFile)('curl',['-fsSL','--retry','2','--connect-timeout','15','--max-time','60',
    'https://cdn.jsdelivr.net/npm/d3@7.9.0/LICENSE','-o',d3License]);
}
manifest.d3={version:'7.9.0',sha256:sha256(await readFile(d3)),bytes:(await stat(d3)).size};
manifest.fonts=await stageFonts(root);
const snapshot=extractSnapshot(await readFile(previous,'utf8'));
validateSnapshot(snapshot);
manifest.graph={nodes:snapshot.nodes.length,links:snapshot.links.length,
  entities:snapshot.nodes.filter(n=>n.type==='Entity').length,semanticRelations:snapshot.links.filter(l=>l.edge_class==='semantic').length,
  topologyHash:snapshot.topologyHash,semanticPositions:Object.keys(snapshot.semanticPositions||{}).length};
const store=new GraphStore(join(root,'native/ladybug-driver'),join(root,'graph.lbdb'));
try {
  await store.initialize(snapshot);
  const restored=await store.read();
  await writeFile(join(root,'index.html'),await render(restored,assets));
  manifest.graph.roundTripHash=sha256(JSON.stringify(restored));
}finally {await store.close();}
await writeFile(join(root,'provenance.json'),JSON.stringify(manifest,null,2));
await writeFile(join(root,'package.json'),JSON.stringify({private:true,type:'module',scripts:{start:'node app/server.mjs'}},null,2));
console.log(JSON.stringify({root,graph:manifest.graph,frontendBytes:manifest.upstreamAssets.reduce((s,f)=>s+f.bytes,0)+manifest.d3.bytes}));
