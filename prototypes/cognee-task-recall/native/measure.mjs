import { readFile, writeFile, stat } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';

const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/native-comparison');
const report=JSON.parse(await readFile(join(root,'comparison.json')));
const f=JSON.parse(await readFile(join(root,'fixture.json')));
const expectedNodes=f.entities.map(e=>[e.id,e.name,e.type,e.description]);
const expectedEdges=f.edges.map(e=>[e.id,e.source,e.target,e.relation,e.evidence,JSON.stringify(e.runIds)]);
const canonical=rows=>rows.map(r=>JSON.stringify(r)).sort();
const digest=rows=>createHash('sha256').update(JSON.stringify(canonical(rows))).digest('hex');
const definitions={
  sqlite:{files:['sqlite-driver'],notes:'Includes statically compiled sqlite-vec; uses macOS system SQLite 3.54.0. Bundled SQLite adds size unless reused from Host.'},
  cozo:{files:['cozo-driver','vendor/libcozo_c.dylib'],notes:'CozoDB 0.7.6 native C ABI; wrapper is C++.'},
  ladybug:{files:['ladybug-driver','vendor/liblbug.0.22.3.dylib'],
    external:['/Users/wangxin/.lbdb/extension/0.22.0/osx_arm64/vector/libvector.lbug_extension','/Users/wangxin/.lbdb/extension/0.22.0/osx_arm64/fts/libfts.lbug_extension'],notes:'Ladybug 0.22.3 C ABI + vector and FTS extensions. Does not include Explorer.'},
  helix:{files:['helix/node_modules/@helix-db/helix-db-embedded/prebuilds/darwin-arm64/libhelixdb_uniffi.dylib'],
    notes:'Published Rust UniFFI engine 0.3.3 only. JS SDK, FFI glue and Node runtime are additional for the tested binding; direct Rust build size is unmeasured.'},
};
for(const engine of report.engines) {
  const spec=definitions[engine.name];
  const files=[...spec.files.map(p=>join(root,p)),...(spec.external||[])];
  engine.distribution={notes:spec.notes,files:[]};
  for(const path of files) {
    const content=await readFile(path);
    engine.distribution.files.push({path,bytes:content.length,gzipBytes:gzipSync(content).length,
      sha256:createHash('sha256').update(content).digest('hex')});
  }
  engine.distribution.rawBytes=engine.distribution.files.reduce((s,f)=>s+f.bytes,0);
  engine.distribution.gzipBytes=engine.distribution.files.reduce((s,f)=>s+f.gzipBytes,0);
  engine.roundTrip={nodes:digest(engine.graph.nodes)===digest(expectedNodes),edges:digest(engine.graph.edges)===digest(expectedEdges),
    mentionCount:engine.graph.mentions.length===f.mentions.length,sharedEntityIdsStable:engine.graph.nodes.every(n=>expectedNodes.some(e=>e[0]===n[0]))};
  for(const lane of ['semantic','lexical','graph']) {
    const values=engine.queries.map(q=>q.timings[lane]).sort((a,b)=>a-b);
    engine[`${lane}Timing`]={median:values[Math.floor(values.length/2)],max:values.at(-1),note:'Includes IPC/SDK and custom traversal; excludes query embedding generation.'};
  }
  assert(Object.values(engine.roundTrip).every(Boolean),`${engine.name}: graph export differs from input`);
  // Check actual vector nearest-neighbor answers against an independent exact cosine calculation.
  let correct=0;
  for(const q of engine.queries) {
    const vector=f.queries.find(v=>v.id===q.id).vector;
    const cosine=c=>{
      let dot=0,a=0,b=0;
      for(let i=0;i<vector.length;++i){dot+=vector[i]*c.vector[i];a+=vector[i]**2;b+=c.vector[i]**2;}
      return dot/Math.sqrt(a*b);
    };
    const ranked=f.chunks.map(c=>[c.id,cosine(c)]).sort((a,b)=>b[1]-a[1]);
    if(q.raw.semantic[0][0]===ranked[0][0])correct++;
  }
  engine.exactCosineTop1Agreement={queries:f.queries.length,matched:correct};
  assert.equal(correct,f.queries.length,`${engine.name}: native vector search disagrees with exact reference`);
}
report.measurement={platform:'macOS arm64',sizeUnit:'decimal bytes',modelWeightsIncluded:false,
  gzipMethod:'Sum of gzip(content) per native file; not signed app/installer size',
  graphChecks:'Full nodes and edges including properties compared against exported input; exact-cosine reference independent of native engines',
  cogneeCoreDependencyRecordBytes:1069563886,cogneeDependencyEstimateNotes:'167 installed distributions reachable without gliner extra; excludes interpreter and non-RECORD files. Not a release bundle.',
  cogneeRuntimeDatabaseBytes:12816840,cogneeIngestionSeconds:323.3577915420756,
  extractionCost:'All four alternatives reuse the same Cognee extraction receipts; 0 fresh LLM calls. Indexing seconds exclude extraction and precomputed embeddings.',
  comparisonCaveat:'Native FTS uses identical CJK bigram preprocessing; Cognee baseline used its default tokenizer. Graph traversal and vector search implementations differ. These are pipeline smoke results, not controlled framework accuracy rankings.'};
await writeFile(join(root,'comparison.json'),JSON.stringify(report,null,2));
const view={...report,engines:report.engines.map(e=>({...e,update:{checks:e.update.checks,updateSeconds:e.update.updateSeconds,workaround:e.update.workaround}}))};
await writeFile(join(root,'view.json'),JSON.stringify(view));
await writeFile(join(root,'documents.json'),JSON.stringify(f.documents.map(d=>({runId:d.runId,taskTitle:d.taskTitle,text:d.text}))));
for(const e of report.engines)console.log(JSON.stringify({name:e.name,status:e.status,nativeMB:e.distribution.rawBytes/1e6,gzipMB:e.distribution.gzipBytes/1e6,databaseMB:e.storageBytes/1e6,roundTrip:e.roundTrip,exactCosine:e.exactCosineTop1Agreement}));
