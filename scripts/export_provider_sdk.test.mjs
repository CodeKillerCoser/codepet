import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,readdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {exportProviderSdk} from './export_provider_sdk.mjs';

test('contract-only export closes every local schema reference and derives RPC envelopes from methods/events',async()=>{
  let output=await mkdtemp(path.join(os.tmpdir(),'provider-contract-'));
  await exportProviderSdk(output);
  const packageRoot=output; output=path.join(output,'v1');
  const files=await readdir(output,{recursive:true});
  assert.ok(files.every(name=>!/(fixtures|gateway|channel|Cargo|\.rs$|\.ts$|cp-sdk-gen)/.test(name)));
  const documents=new Map();
  for(const file of files.filter(name=>name.endsWith('.json'))) documents.set(path.resolve(output,file),JSON.parse(await readFile(path.join(output,file),'utf8')));
  function walk(value,source){
    if(!value||typeof value!=='object')return;
    if(value.$ref){
      const [file,pointer='']=value.$ref.split('#');
      assert.ok(!/^https?:/.test(file),'published references must resolve locally');
      const targetPath=file?path.resolve(path.dirname(source),file):source;
      let target=documents.get(targetPath);assert.ok(target,`${source}: missing ${file}`);
      for(const part of pointer.split('/').slice(1)){target=target[part.replaceAll('~1','/').replaceAll('~0','~')];assert.notEqual(target,undefined,`${source}: missing ${value.$ref}`);}
    }
    for(const child of Object.values(value))if(Array.isArray(child))child.forEach(v=>walk(v,source));else walk(child,source);
  }
  for(const [source,value]of documents)walk(value,source);
  const wire=documents.get(path.join(output,'protocol/sdk/rpc/envelope.schema.json'));
  const methods=documents.get(path.join(output,'protocol/sdk/provider/methods.json')).methods;
  const events=documents.get(path.join(output,'protocol/sdk/provider/events.json')).events;
  assert.equal(wire.$defs.Request.oneOf.length,methods.length);assert.equal(wire.$defs.Event.oneOf.length,events.length);
  const profiles=documents.get(path.join(output,'protocol/sdk/transport/profiles.json')).profiles.map(p=>p.id).sort();
  assert.deepEqual(documents.get(path.join(output,'protocol/manifest.schema.json')).properties.transport.enum.sort(),profiles);
  await assert.rejects(exportProviderSdk(packageRoot),/empty output/);
});
test('export refuses to mix existing private resources with the public SDK',async()=>{
  const output=await mkdtemp(path.join(os.tmpdir(),'provider-contract-private-'));
  await writeFile(path.join(output,'private-source.rs'),'private');
  await assert.rejects(exportProviderSdk(output),/empty output/);
});
