import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { sha256, extractSnapshot, readAssignment, validateSnapshot } from './snapshot.mjs';

const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/ladybug-cognee-renderer');
const {GraphStore}=await import(pathToFileURL(join(root,'app/store.mjs')));
const {render}=await import(pathToFileURL(join(root,'app/render.mjs')));
const provenance=JSON.parse(await readFile(join(root,'provenance.json'),'utf8'));
const checks={};
const store=new GraphStore(join(root,'native/ladybug-driver'),join(root,'graph.lbdb'));
let source;
try {
  source=await store.read();
  const upstream=extractSnapshot(await readFile(provenance.input.path,'utf8'));
  assert.deepEqual(source,upstream);
  checks.completeDatabaseRoundTrip=true;
  const original=source.nodes.find(n=>n.type==='Entity'&&n.name==='http');
  assert(original,'expected real HTTP entity');
  try {
    const changed={...original,name:'HTTP · NATIVE_RENDER_UPDATE_CHECK',description:original.description+' [native update]'};
    await store.replaceNode(changed);
    const reread=await store.read();
    assert.equal(reread.nodes.find(n=>n.id===original.id).name,changed.name);
    assert.equal(reread.schemaGraph.instance_index[original.id].name,changed.name);
    const html=await render(reread,join(root,'assets'));
    assert(html.includes('NATIVE_RENDER_UPDATE_CHECK'));
    checks.propertyUpdateReadBackAndRendered=true;
  } finally {await store.replaceNode(original);}
  assert.deepEqual(await store.read(),source);
  checks.restored=true;
} finally {await store.close();}

for(const asset of provenance.upstreamAssets) {
  assert.equal(sha256(await readFile(join(root,'assets',asset.source.split('/').at(-1)))),asset.sha256);
}
checks.originalCogneeAssetsUnchanged=true;
assert.deepEqual(readAssignment('const x = {"text":"[\\\";]"}; evil()', 'const x = '),{text:'[";]'});
assert.throws(()=>validateSnapshot({...source,nodes:source.nodes.slice(1)}));
checks.parserDoesNotEvaluateJSAndTopologyMismatchRejected=true;
const attack=structuredClone(source);
attack.nodes[0].name='</script><script>globalThis.CODEPET_UNSAFE=true</script>';
assert(!(await render(attack,join(root,'assets'))).includes(attack.nodes[0].name));
checks.scriptTerminatorEscaped=true;

// Start the copied runtime with an empty executable search path. It can only spawn
// the explicitly named native driver; neither Python nor a package manager is available.
const emptyPath=join(root,'empty-bin');await mkdir(emptyPath,{recursive:true});
const child=spawn(process.execPath,[join(root,'app/server.mjs'),root,'0'],{
  cwd:root,env:{PATH:emptyPath},stdio:['ignore','pipe','pipe']});
let stderr='';child.stderr.on('data',data=>{stderr+=data;});
const exited=new Promise(resolve=>child.once('exit',(code,signal)=>resolve({code,signal})));
try {
  const ready=await Promise.race([
    new Promise((resolve,reject)=>{
      const lines=createInterface({input:child.stdout});
      lines.once('line',line=>{try{resolve(JSON.parse(line));}catch(e){reject(e);}});
      child.once('error',reject);
    }),new Promise((_,reject)=>setTimeout(()=>reject(new Error(`cold-start timeout: ${stderr}`)),15000).unref()),
  ]);
  const html=await fetch(ready.url).then(r=>{assert(r.ok);return r.text();});
  assert.deepEqual(readAssignment(html,'var nodes = '),source.nodes);
  const reread=await fetch(new URL('graph.json',ready.url)).then(r=>r.json());
  assert.deepEqual(reread,source);
  const scripts=[...html.matchAll(/<script[^>]+src="([^"]+)"/g)].map(m=>m[1]);
  assert.deepEqual(scripts,['/assets/d3.v7.9.0.min.js']);
  assert.equal(sha256(Buffer.from(await fetch(new URL(scripts[0],ready.url)).then(r=>r.arrayBuffer()))),provenance.d3.sha256);
  assert(!html.includes('https://fonts.googleapis.com'));
  const css=await fetch(new URL('/assets/inter.css',ready.url)).then(r=>r.text());
  assert(!css.includes('https://'));
  for(const font of provenance.fonts.files) {
    const data=Buffer.from(await fetch(new URL('/assets/'+font.name,ready.url)).then(r=>r.arrayBuffer()));
    assert.equal(sha256(data),font.sha256);
  }
  checks.coldStartedFromCopiedRuntimeWithEmptyPATH=true;
  checks.D3AndInterFontsAvailableLocally=true;
  checks.servedDataExactlyMatchesLadybug=true;
  checks.runtime={node:process.execPath,driver:join(root,'native/ladybug-driver'),pythonExecutableAvailable:false};
} finally {child.kill('SIGTERM');await exited;}
await writeFile(join(root,'verification.json'),JSON.stringify({createdAt:new Date().toISOString(),checks},null,2));
console.log(JSON.stringify(checks));
