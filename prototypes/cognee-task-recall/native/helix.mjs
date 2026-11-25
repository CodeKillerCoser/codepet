import { pathToFileURL } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { tokens, unique } from './ipc.mjs';

export class Helix {
  constructor(root,path,audit) { this.root=root;this.path=path;this.audit=audit;this.ids=new Map(); }
  async open() {
    await mkdir(this.path,{recursive:true});
    process.env.HELIX_TELEMETRY_LEVEL='off';
    this.sdk=await import(pathToFileURL(`${this.root}/helix/node_modules/@helix-db/helix-db/dist/index.js`));
    this.client=await this.sdk.Client.embedded({kind:'disk',root:this.path,database:'probe'});
  }
  async send(traversal,write=false) {
    const started=performance.now();
    const batch=(write?this.sdk.writeBatch():this.sdk.readBatch()).varAs('rows',traversal).returning(['rows']);
    try {
      const result=await this.client.query(batch.toQueryRequest()).send();
      this.audit.push({ok:true,operation:traversal.toJSON(),result,seconds:(performance.now()-started)/1000});
      return result.rows;
    } catch(error) {
      this.audit.push({ok:false,error:String(error),seconds:(performance.now()-started)/1000});throw error;
    }
  }
  g() { return this.sdk.g(); }
  async add(label,id,properties) {
    const result=await this.send(this.g().addN(label,{id,...properties}),true);
    this.ids.set(id,result[0].$id);
  }
  vector(v) { return this.sdk.PropertyValue.f32Array(v); }
  async addChunk(c) {
    await this.add('Chunk',c.id,{run_id:c.runId,text:c.text,lexical:tokens(c.text).join(' '),vector:this.vector(c.vector)});
  }
  async addEntity(e) {
    await this.add('Entity',e.id,{name:e.name,type:e.type,description:e.description,vector:this.vector(e.vector)});
  }
  async edge(label,source,target,properties={}) {
    await this.send(this.g().n(this.ids.get(source)).addE(label,this.ids.get(target),properties),true);
  }
  async init(f) {
    await this.open();
    for(const c of f.chunks) await this.addChunk(c);
    for(const e of f.entities) await this.addEntity(e);
    for(const [c,e] of f.mentions) await this.edge('Mentions',c,e);
    for(const e of f.edges) await this.edge('Related',e.source,e.target,{id:e.id,source:e.source,target:e.target,relation:e.relation,evidence:e.evidence,run_ids:JSON.stringify(e.runIds)});
    const receipts=[];
    for(const label of ['Chunk','Entity']) receipts.push(await this.send(this.g().createVectorIndexNodes(label,'vector',1024,'cosine'),true));
    receipts.push(await this.send(this.g().createTextIndexNodes('Chunk','lexical'),true));
    this.indexReceipts=receipts;
    for(const receipt of receipts) {
      if(!receipt.operation_id) continue;
      const deadline=Date.now()+55000;
      let operation;
      while(Date.now()<deadline) {
        operation=await this.send(this.g().getIndexOperation(receipt.operation_id));
        if(['succeeded','completed','ready','active'].includes(operation.status)) break;
        if(['blocked','failed','aborted'].includes(operation.status)) throw new Error(JSON.stringify(operation));
        await new Promise(resolve=>setTimeout(resolve,100));
      }
      if(!['succeeded','completed','ready','active'].includes(operation?.status)) throw new Error(`index activation timeout: ${JSON.stringify(operation)}`);
    }
  }
  async semantic(v,table='chunk') {
    const rows=await this.send(this.g().vectorSearchNodes(table==='chunk'?'Chunk':'Entity','vector',v,8).valueMap(['id','$distance']));
    return rows.map(r=>[r.id,r.$distance]);
  }
  async lexical(query) {
    const rows=await this.send(this.g().textSearchNodes('Chunk','lexical',unique(tokens(query)).join(' '),8).valueMap(['id','$score']));
    return rows.map((r,i)=>[r.id,r.$score??i]);
  }
  async graph(seeds) {
    const ids=seeds.map(id=>this.ids.get(id));
    const reached=await this.send(this.g().n(ids).both('Related').valueMap(['id']));
    const all=unique([...seeds,...reached.map(r=>r.id)]), rows=[];
    // Resolve graph adjacency in the engine; retain explicit chunk provenance for Run results.
    for(const entity of all) {
      const mentions=await this.send(this.g().n(this.ids.get(entity)).in('Mentions').valueMap(['run_id']));
      rows.push(...mentions.map(r=>[r.run_id,entity]));
    }
    return rows;
  }
  async export() {
    const nodes=await this.send(this.g().nWithLabel('Entity').valueMap(['$id','id','name','type','description']));
    const edges=await this.send(this.g().eWithLabel('Related').valueMap(['id','source','target','relation','evidence','run_ids']));
    const mentions=[];
    for(const e of nodes) {
      this.ids.set(e.id,e.$id);
      const chunks=await this.send(this.g().n(e.$id).in('Mentions').valueMap(['run_id']));
      mentions.push(...chunks.map(c=>[c.run_id,e.id]));
    }
    return {nodes:nodes.map(n=>[n.id,n.name,n.type,n.description]).sort((a,b)=>a[0].localeCompare(b[0])),
      edges:edges.map(e=>[e.id,e.source,e.target,e.relation,e.evidence,e.run_ids]).sort((a,b)=>a[0].localeCompare(b[0])),mentions};
  }
  async control(f,revision) {
    const next=revision==='after';
    if(next) {
      await this.send(this.g().n(this.ids.get('native-old')).drop(),true);
      await this.send(this.g().n(this.ids.get('native-control')).drop(),true);
    } else {
      await this.addChunk({id:'native-keeper',runId:'native-keeper',text:'Shared entity keeper',vector:f.chunks[0].vector});
      await this.addEntity({id:'native-shared',name:'Shared',type:'Control',description:'',vector:f.chunks[0].vector});
      await this.edge('Mentions','native-keeper','native-shared');
    }
    const name=next?'native-new':'native-old';
    await this.addChunk({id:'native-control',runId:'native-control',text:next?'codepetnewfixture9ad2':'codepetoldfixture7fe4',vector:f.chunks[next?1:0].vector});
    await this.addEntity({id:name,name:next?'New':'Old',type:'Control',description:'',vector:f.chunks[0].vector});
    for(const entity of [name,'native-shared']) await this.edge('Mentions','native-control',entity);
    await this.edge('Related',name,'native-shared',{id:'native-edge',source:name,target:'native-shared',relation:'uses',evidence:'synthetic control',run_ids:'[]'});
  }
  async cleanup() {
    await this.send(this.g().n(['native-control','native-keeper','native-new','native-shared'].map(s=>this.ids.get(s))).drop(),true);
  }
  async close() { await this.client.close(); }
}
