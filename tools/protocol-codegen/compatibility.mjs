import { resolveSchemaPath } from "./schema-paths.mjs";
// Compile the same functional IDL graph into validation data for all language targets.
export function compatibilityGraph(record, model) {
  const records = new Map();
  function include(r) { if (records.has(r.packageConfig.id)) return; records.set(r.packageConfig.id,r); for (const id of r.packageConfig.dependencies) include(model.recordsById.get(id)); }
  include(record);
  const definitions = {};
  const key = (r,name) => `${r.packageConfig.layer}:${name}`;
  function normalized(node,r) {
    if (Array.isArray(node)) return node.map(n=>normalized(n,r));
    if (!node || typeof node !== 'object') return node;
    const out={};
    for (const [k,v] of Object.entries(node)) {
      if (['description','$id','$schema','title'].includes(k) || k.startsWith('x-codepet')) continue;
      if (k === '$ref') {
        const [file,pointer]=v.split('#');
        const target=file ? model.schemasByPath.get(resolveSchemaPath(r.schemaPath,file)) : r;
        if (!target) throw new Error(`Missing compatibility graph ref ${v}`);
        out.ref=key(target,pointer.split('/').at(-1));
      } else out[k]=normalized(v,r);
    }
    if (node.oneOf?.every(v=>v.$ref)) {
      const variants=node.oneOf.map(v=>{const n=normalized(v,r);return {ref:n.ref,node:definitions[n.ref]};});
      // Definitions may not have been visited yet; resolve once the graph is complete.
      out.unionRefs=variants.map(v=>v.ref);
    }
    return out;
  }
  for (const r of records.values()) for (const [name,node] of Object.entries(r.schema.$defs)) definitions[key(r,name)]=normalized(node,r);
  for (const node of Object.values(definitions)) if(node.unionRefs) {
    const variants=node.unionRefs.map(ref=>[ref,definitions[ref]]);
    const fields=Object.keys(variants[0][1].properties??{});
    const field=fields.find(f=>variants.every(([,v])=>v.required?.includes(f)&&v.properties?.[f]?.ref&&definitions[v.properties[f].ref]?.enum?.length===1));
    if(field) node.discriminator={field,variants:Object.fromEntries(variants.map(([ref,v])=>[definitions[v.properties[field].ref].enum[0],ref]))};
    delete node.unionRefs;
  }
  return {definitions,key:(name)=>key(record,name)};
}

export function typescriptValidation(graph) {
return `
const protocolSchemas: Record<string, any> = ${JSON.stringify(graph.definitions)};
/** Validate known fields; unknown fields are ignored and future receive values retained. */
export function validateProtocolValue(type: string, value: unknown, allowUnknown = true): void {
  function visit(n: any, v: any, path: string): void {
    const fail = () => { throw new TypeError('Invalid protocol value at ' + path); };
    if(n.ref) return visit(protocolSchemas[n.ref],v,path);
    if(n.discriminator) {
      if(!v || typeof v !== 'object' || Array.isArray(v) || typeof v[n.discriminator.field] !== 'string') return fail();
      const ref=n.discriminator.variants[v[n.discriminator.field]];
      if(ref) return visit(protocolSchemas[ref],v,path);
      if(allowUnknown) return; return fail();
    }
    if(n.oneOf) { let count=0; for(const variant of n.oneOf) { try { visit(variant,v,path);count++; } catch {} } if(count!==1) fail(); return; }
    if(n.enum && !n.enum.includes(v) && !(allowUnknown && n.enum.length>1 && typeof v==='string')) fail();
    if(n.type==='null' && v!==null) fail();
    if(n.type==='boolean' && typeof v!=='boolean') fail();
    if(n.type==='integer' && (!Number.isSafeInteger(v) || n.minimum!==undefined&&v<n.minimum || n.maximum!==undefined&&v>n.maximum)) fail();
    if(n.type==='string' && (typeof v!=='string' || n.minLength!==undefined&&Array.from(v).length<n.minLength || n.maxLength!==undefined&&Array.from(v).length>n.maxLength || n.pattern!==undefined&&!new RegExp(n.pattern).test(v))) fail();
    if(n.type==='array') { if(!Array.isArray(v)) return fail(); if(n.minItems!==undefined&&v.length<n.minItems) fail(); if(n.uniqueItems&&new Set(v.map(x=>JSON.stringify(x))).size!==v.length) fail(); v.forEach((x,i)=>visit(n.items,x,path+'['+i+']')); }
    if(n.type==='object') { if(!v || typeof v!=='object' || Array.isArray(v)) return fail(); for(const k of n.required??[]) if(!Object.hasOwn(v,k)) fail(); for(const [k,x] of Object.entries(v)) { if(n.properties?.[k]) visit(n.properties[k],x,path+'.'+k); else if(typeof n.additionalProperties==='object') visit(n.additionalProperties,x,path+'.'+k); } }
  }
  if(!protocolSchemas[type]) throw new TypeError('Unknown protocol type: '+type);
  visit(protocolSchemas[type],value,type);
}
/** Decode a receive value while preserving unknown enum and union values verbatim. */
export function decodeProtocolValue(type: string, value: unknown): unknown { validateProtocolValue(type,value,true); return value; }
/** Sending does not promote preserved unknown choices into executable requests. */
export function encodeProtocolValue(type: string, value: unknown): string { validateProtocolValue(type,value,false); return JSON.stringify(value); }
`;
}

export function typescriptVersionNegotiation() {
  return `
/** Respond with this side's range and the highest mutually supported revision. */
export function negotiateProtocolVersion(local: ProtocolVersionOffer, peer: ProtocolVersionOffer): ProtocolVersionSelection {
  validateProtocolValue('core:ProtocolVersionOffer',local,false);
  validateProtocolValue('core:ProtocolVersionOffer',peer,false);
  if(local.minVersion>local.version || peer.minVersion>peer.version) throw new TypeError('invalid_protocol_range');
  const selectedVersion=Math.min(local.version,peer.version);
  if(selectedVersion<Math.max(local.minVersion,peer.minVersion)) throw new TypeError('unsupported_protocol_version');
  return {version:local.version,minVersion:local.minVersion,selectedVersion};
}
export function validateProtocolSelection(local: ProtocolVersionOffer, peer: ProtocolVersionSelection): void {
  validateProtocolValue('core:ProtocolVersionSelection',peer,false);
  if(negotiateProtocolVersion(local,peer).selectedVersion!==peer.selectedVersion) throw new TypeError('invalid_protocol_selection');
}
`;
}
