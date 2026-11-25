import { readFile, writeFile, mkdir, stat, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
import { SQLite } from './sqlite.mjs';
import { Cozo } from './cozo.mjs';
import { Ladybug } from './ladybug.mjs';
import { Helix } from './helix.mjs';
import { unique } from './ipc.mjs';

export function fuse(routes, documents, workspace) {
  const lanes={...routes,recency:[...documents].sort((a,b)=>b.updatedAt-a.updatedAt).map(d=>d.runId),
    workspace:documents.filter(d=>d.workspace===workspace).map(d=>d.runId)};
  const weights={semantic:1,lexical:.7,graph:.4,recency:.04,workspace:.05};
  const allowed=new Set(Object.values(routes).flat()),scores=new Map();
  for(const [lane,ids] of Object.entries(lanes)) for(const [i,id] of ids.entries()) {
    if(allowed.has(id)) scores.set(id,(scores.get(id)||0)+weights[lane]/(11+i));
  }
  return [...scores].sort((a,b)=>b[1]-a[1]||a[0].localeCompare(b[0])).map(r=>r[0]);
}
export function graphRanks(rows) {
  const scores=new Map();
  for(const [run,entity] of rows) {
    if(!scores.has(run)) scores.set(run,new Set());
    scores.get(run).add(entity);
  }
  return [...scores].sort((a,b)=>b[1].size-a[1].size||a[0].localeCompare(b[0])).map(r=>r[0]);
}
export function metrics(queries,lane) {
  const positives=queries.filter(q=>q.expectedRunIds.length);
  const ranks=positives.map(q=>q[lane].findIndex(id=>q.expectedRunIds.includes(id))+1);
  return {cases:positives.length,hitAt1:ranks.filter(r=>r===1).length,
    hitAt3:ranks.filter(r=>r>0&&r<=3).length};
}
async function bytes(path) {
  const s=await stat(path);
  if(s.isFile()) return s.size;
  let total=0;
  for(const p of await readdir(path)) total+=await bytes(join(path,p));
  return total;
}
async function embed(inputs,root) {
  const vectors=[];
  for(let i=0;i<inputs.length;i+=8) {
    const batch=inputs.slice(i,i+8),started=performance.now();
    const response=await fetch('http://127.0.0.1:11234/v1/embeddings',{
      method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({model:'Qwen3-Embedding-0.6B-8bit',input:batch,encoding_format:'float'}),
      signal:AbortSignal.timeout(60000),
    });
    if(!response.ok) throw new Error(`embedding ${response.status}: ${await response.text()}`);
    const result=await response.json();
    const rows=result.data.sort((a,b)=>a.index-b.index);
    if(rows.length!==batch.length||rows.some(r=>r.embedding.length!==1024)) throw new Error('invalid vectors');
    vectors.push(...rows.map(r=>r.embedding));
    await writeFile(join(root,`embedding-${Date.now()}-${i}.json`),JSON.stringify({inputs:batch,usage:result.usage,
      dimensions:rows.map(r=>r.embedding.length),seconds:(performance.now()-started)/1000}));
  }
  return vectors;
}
async function controls(engine,f) {
  await engine.control(f,'before');
  const before=await engine.export();
  const oldBefore=await engine.lexical('codepetoldfixture7fe4');
  const started=performance.now();
  await engine.control(f,'after');
  const updateSeconds=(performance.now()-started)/1000;
  const oldAfter=await engine.lexical('codepetoldfixture7fe4');
  const newAfter=await engine.lexical('codepetnewfixture9ad2');
  const vectorHitsAfter=await engine.semantic(f.chunks[1].vector);
  const updatedVectorHit=vectorHitsAfter.find(r=>r[0]==='native-control');
  const after=await engine.export();
  let actual;
  if(engine instanceof SQLite) actual=JSON.parse((await engine.q('SELECT vec_to_json(embedding) FROM chunk_vec WHERE id=10001'))[0][0]);
  else if(engine instanceof Cozo) actual=(await engine.q('?[vector] := *chunk{id:"native-control",vector}'))[0][0];
  else if(engine instanceof Ladybug) actual=(await engine.q('MATCH (c:Chunk {id:"native-control"}) RETURN c.vector'))[0][0];
  else actual=(await engine.send(engine.g().n(engine.ids.get('native-control')).valueMap(['vector'])))[0].vector;
  const checks={oldKeywordInitiallyPresent:oldBefore.some(r=>r[0]==='native-control'),
    oldKeywordRemoved:!oldAfter.some(r=>r[0]==='native-control'),
    newKeywordPresent:newAfter.some(r=>r[0]==='native-control'),
    vectorReplaced:Array.isArray(actual)&&actual.length===1024&&actual.every((v,i)=>Math.abs(v-f.chunks[1].vector[i])<.00001),
    vectorIndexUpdated:!!updatedVectorHit&&Math.abs(updatedVectorHit[1])<.00001,
    oldEntityRemoved:!after.nodes.some(n=>n[0]==='native-old'),
    newEntityAdded:after.nodes.some(n=>n[0]==='native-new'),
    oldRelationRemoved:!after.edges.some(e=>e[1]==='native-old'||e[2]==='native-old'),
    newRelationAdded:after.edges.some(e=>e[1]==='native-new'),
    sharedEntityRetained:after.nodes.some(n=>n[0]==='native-shared'),
    otherRunSharedLinkRetained:after.mentions.some(r=>r[0]==='native-keeper'&&r[1]==='native-shared')};
  let workaround;
  if(engine instanceof Cozo&&!checks.oldKeywordRemoved) {
    const started=performance.now();
    await engine.q('::fts drop chunk:fts');
    await engine.q('::fts create chunk:fts {extractor:lexical,tokenizer:Whitespace,filters:[Lowercase]}');
    workaround={kind:'rebuild whole FTS index',seconds:(performance.now()-started)/1000,
      oldKeywordRemoved:!(await engine.lexical('codepetoldfixture7fe4')).some(r=>r[0]==='native-control')};
  }
  await engine.cleanup();
  const restored=await engine.export();
  checks.fixtureRestored=restored.nodes.length===f.entities.length&&restored.edges.length===f.edges.length&&restored.mentions.length===f.mentions.length;
  return {checks,updateSeconds,before,after,workaround,vectorHitsAfter};
}
async function main() {
  const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/native-comparison');
  const f=JSON.parse(await readFile(join(root,'fixture.json')));
  const missing=f.entities.filter(e=>!e.vector);
  if(missing.length) {
    const values=await embed(missing.map(e=>e.name),root);
    missing.forEach((e,i)=>{e.vector=values[i];});
  }
  if(!f.queries.every(q=>q.vector)) {
    const values=await embed(f.queries.map(q=>q.query),root);
    f.queries.forEach((q,i)=>{q.vector=values[i];});
  }
  await writeFile(join(root,'fixture.json'),JSON.stringify(f));
  const preparedHash=createHash('sha256').update(JSON.stringify(f)).digest('hex');
  const report={createdAt:new Date().toISOString(),source:f.source,fixtureHash:f.fixtureHash,preparedHash,
    reusedExtraction:true,freshLLMCalls:0,missingEntityVectorsRegenerated:missing.length,
    documents:f.documents.map(({text,activity,criteria,...d})=>d),chunks:f.chunks.length,
    entities:f.entities.length,relations:f.edges.length,cognee:{metrics:f.baselineMetrics},engines:[]};
  try { report.engines=JSON.parse(await readFile(join(root,'comparison.json'))).engines; } catch {}
  const selected=process.argv.slice(3);
  for(const [name,Backend] of [['sqlite',SQLite],['cozo',Cozo],['ladybug',Ladybug],['helix',Helix]]) {
    if(selected.length&&!selected.includes(name)) continue;
    const folder=join(root,`${name}-${Date.now()}`);await mkdir(folder);
    const audit=[],result={name,folder,queries:[]};report.engines=report.engines.filter(e=>e.name!==name);report.engines.push(result);
    const engine=new Backend(root,join(folder,'database'),audit);
    let closed=false;
    try {
      const started=performance.now();await engine.init(f);result.ingestionSeconds=(performance.now()-started)/1000;
      console.log(JSON.stringify({engine:name,phase:'ingested',seconds:result.ingestionSeconds}));
      const map=new Map(f.chunks.map(c=>[c.id,c.runId]));
      for(const query of f.queries) {
        const q={id:query.id,query:query.query,expectedRunIds:query.expectedRunIds,timings:{},raw:{}};
        for(const lane of ['semantic','lexical','graph']) {
          const start=performance.now();
          if(lane==='graph') {
            q.raw.seeds=await engine.semantic(query.vector,'entity');
            q.raw.graph=await engine.graph(q.raw.seeds.slice(0,4).map(r=>r[0]));
            q.graph=graphRanks(q.raw.graph);
          } else {
            q.raw[lane]=await engine[lane](lane==='semantic'?query.vector:query.query);
            q[lane]=unique(q.raw[lane].map(r=>map.get(r[0])).filter(Boolean));
          }
          q.timings[lane]=(performance.now()-start)/1000;
        }
        q.fused=fuse({semantic:q.semantic,lexical:q.lexical,graph:q.graph},f.documents,f.workspace);
        result.queries.push(q);
      }
      result.graph=await engine.export();
      result.metrics=Object.fromEntries(['semantic','lexical','graph','fused'].map(l=>[l,metrics(result.queries,l)]));
      result.update=await controls(engine,f);
      result.graphAfterCleanup=await engine.export();
      await engine.close();closed=true;
      result.storageBytes=await bytes(folder);
      result.reopen={};const reader=new Backend(root,join(folder,'database'),audit);
      try {
        if(reader.open) await reader.open();
        const reopened=await reader.export();
        result.reopen.graphSurvived=reopened.nodes.length===f.entities.length&&reopened.edges.length===f.edges.length;
        result.reopen.lexicalSurvived=(await reader.lexical('MCP')).length>0;
        result.reopen.vectorSurvived=(await reader.semantic(f.queries[0].vector)).length>0;
      } finally { await reader.close(); }
      result.status=Object.values(result.update.checks).every(Boolean)&&Object.values(result.reopen).every(Boolean)?'passed':'failed_checks';
    } catch(error) { result.status='error';result.error=String(error.stack||error);console.log(JSON.stringify({engine:name,error:String(error)})); }
    finally {
      if(!closed) await engine.close().catch(e=>{result.closeError=String(e);});
      await writeFile(join(folder,'audit.json'),JSON.stringify(audit));
      await writeFile(join(folder,'result.json'),JSON.stringify(result,null,2));
      await writeFile(join(root,'comparison.json'),JSON.stringify(report,null,2));
    }
    console.log(JSON.stringify({engine:name,status:result.status,metrics:result.metrics,checks:result.update?.checks,storageBytes:result.storageBytes}));
  }
}
if(process.argv[1]&&import.meta.url===new URL(`file://${resolve(process.argv[1])}`).href) await main();
