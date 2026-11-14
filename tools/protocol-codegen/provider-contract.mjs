// Canonical Provider IDL loader. Publication, compatibility DTOs and runtime code use this model.
import { readFile } from 'node:fs/promises';
import { loadModularContract, providerPublicationInputs, validateDescriptions } from './modular-contract.mjs';
import path from 'node:path';
import { createHash } from 'node:crypto';
const clone = value => structuredClone(value);
const assert = (value, message) => { if (!value) throw new Error(`Provider IDL: ${message}`); };
const encode = value => `${JSON.stringify(value, null, 2)}\n`;
export async function loadProviderContract(protocolRoot) {
  const canonical = await loadModularContract(protocolRoot);
  const root = path.join(protocolRoot, `v${canonical.index.version}/provider`);
  const { index, files } = providerPublicationInputs(canonical);
  const entryKeys=['manifestSchema','profiles','transportHandshakeSchema','rpcEnvelopeSchema','commonSchema','agentDomainSchema','providerLifecycleSchema','providerMessagesSchema','methods','events'];
  const expected=new Set(['sdk-manifest.json',...entryKeys.map(key=>index[key])]);
  assert(files.size===expected.size&&[...files.keys()].every(file=>expected.has(file)),'unexpected or missing published IDL file');
  const { profiles, wire } = files.get(index.profiles);
  const limits = files.get(index.transportHandshakeSchema).$defs.ProviderTransportLimits.properties;
  const lifecycle = files.get(index.providerLifecycleSchema)['x-codepet-lifecycle'];
  const constraints=files.get(index.transportHandshakeSchema)['x-codepet-constraints'];
  assert(profiles.length === 3 && new Set(profiles.map(v=>v.id)).size === profiles.length, 'exactly three unique profiles are required');
  for (const [name,value] of Object.entries(limits)) assert(Number.isSafeInteger(value.default) && value.default >= value.minimum && value.default <= value.maximum, `invalid default ${name}`);
  assert(lifecycle.heartbeatIntervalMs > 0 && lifecycle.heartbeatTimeoutMs > lifecycle.heartbeatIntervalMs, 'invalid heartbeat policy');
  const evaluate=(value)=>typeof value==='string'?limits[value]?.default:typeof value==='number'?value:value.sum?value.sum.map(evaluate).reduce((a,b)=>a+b,0):value.multiply?value.multiply.map(evaluate).reduce((a,b)=>a*b,1):NaN;
  for(const rule of constraints){const left=evaluate(rule.left),right=evaluate(rule.right);assert(Number.isFinite(left)&&Number.isFinite(right)&&(['>=','<='].includes(rule.operator))&&(rule.operator==='>='?left>=right:left<=right),`inconsistent transport defaults: ${rule.description}`);}
  assert(wire.byteOrder==='big-endian','unsupported byte order');
  for(const value of [wire.bootstrap.version,wire.bootstrap.confirmation,...Object.values(wire.bootstrap.phases),wire.frame.version,...Object.values(wire.frame.encodings),wire.message.version,wire.message.grant,wire.message.eventAck,...Object.values(wire.message.classes)])assert(Number.isInteger(value)&&value>=0&&value<=255,'wire code must be u8');
  assert(Number.isSafeInteger(wire.bootstrap.timeoutMs)&&wire.bootstrap.timeoutMs>0,'invalid bootstrap timeout');
  const features=wire.bootstrap.features;
  assert(features.length && new Set(features).size===features.length,'duplicate bootstrap features');
  for(const f of [wire.bootstrap.magic,wire.frame.magic])assert(Buffer.byteLength(f)===4,'magic must be four bytes');
  function refs(value,file){
    if(!value||typeof value!=='object')return;
    if(value.$ref){const [relative,pointer='']=value.$ref.split('#');assert(!/^[a-z]+:/i.test(relative),'external schema ref');const target=path.posix.normalize(path.posix.join(path.posix.dirname(file),relative||path.posix.basename(file)));let node=files.get(target);assert(node,`missing reference file ${target}`);for(const part of pointer.split('/').slice(1)){node=node[part.replaceAll('~1','/').replaceAll('~0','~')];assert(node!==undefined,`unresolved ${file}: ${value.$ref}`);}}
    for(const child of Object.values(value))if(Array.isArray(child))child.forEach(v=>refs(v,file));else refs(child,file);
  }
  for(const [file,value]of files){refs(value,file);if(file.endsWith('.schema.json'))validateDescriptions(value,file);}
  const methods=files.get(index.methods).methods,events=files.get(index.events).events;
  const envelope=files.get(index.rpcEnvelopeSchema);
  // JSON-RPC is a supported wire dialect. Reject unsupported shapes instead of silently emitting a different protocol.
  for(const [name,keys]of Object.entries({Request:['jsonrpc','id','method','params'],Event:['jsonrpc','method','params'],SuccessResponse:['jsonrpc','id','result'],ErrorResponse:['jsonrpc','id','error']})){
    const d=envelope.$defs[name];assert(d?.type==='object'&&JSON.stringify(Object.keys(d.properties).sort())===JSON.stringify(keys.sort()),`unsupported ${name} envelope shape`);
    assert(d.properties.jsonrpc.const==='2.0',`unsupported ${name} JSON-RPC version`);
  }
  const hash=createHash('sha256');
  for(const [file,value]of [...files].sort(([a],[b])=>a.localeCompare(b))){hash.update(file);hash.update('\0');hash.update(encode(value));}
  const readme = await readFile(path.join(root,index.guide),'utf8');
  hash.update(readme);const sourceDigest=`sha256:${hash.digest('hex')}`;
  const documentation={heartbeatIntervalSeconds:lifecycle.heartbeatIntervalMs/1000,heartbeatTimeoutSeconds:lifecycle.heartbeatTimeoutMs/1000,bootstrapTimeoutSeconds:wire.bootstrap.timeoutMs/1000,features:wire.bootstrap.features.map(v=>`\`${v}\``).join('、')};
  return {root,index,files,profiles,wire,limits,constraints,lifecycle,methods,events,readme,documentation,sourceDigest,canonical};
}
export function publishProviderContract(model) {
  const {index}=model;const files=new Map([...model.files].map(([name,value])=>[name,clone(value)]));
  files.get('sdk-manifest.json').sourceDigest=model.sourceDigest;
  for(const registry of [index.methods,index.events])files.get(registry).version=index.protocolVersion;
  files.get(index.manifestSchema).properties.transport.enum=model.profiles.map(v=>v.id);
  delete files.get(index.manifestSchema).properties.transport['x-codepet-profile-enum'];
  const envelope=files.get(index.rpcEnvelopeSchema);
  const rebase=ref=>{const [rel,fragment]=ref.$ref.split('#');const dest=path.posix.normalize(path.posix.join(path.posix.dirname(index.methods),rel));return {$ref:`${path.posix.relative(path.posix.dirname(index.rpcEnvelopeSchema),dest)}#${fragment}`,description:ref.description??'关联载荷的 Schema。'};};
  for(const [name,entries,type]of [['Request',model.methods,'request'],['Event',model.events,'payload']]){
    const base=clone(envelope.$defs[name]);envelope.$defs[name]={description:base.description,oneOf:entries.map(entry=>{const d=clone(base);d.description=entry.description;d.properties.method={const:entry.name,description:entry.description};d.properties.params=rebase(entry[type]);return d;})};
  }
  const texts=new Map([...files].map(([name,value])=>[name,encode(value)]));
  texts.set('README.md',model.readme.replace(/\{\{([\w.]+)\}\}/g,(_,key)=>{let v=model;for(const part of key.split('.'))v=v?.[part];assert(['string','number'].includes(typeof v),`unknown README value ${key}`);return String(v);}));
  return texts;
}
export { encode as encodeContractJson };
