import { NativeConnection, tokens, literal as s } from './ipc.mjs';

export class Ladybug {
  constructor(root,path,audit) { this.db=new NativeConnection(`${root}/ladybug-driver`,path,audit); }
  q(sql) { return this.db.query(sql); }
  async open() { for(const extension of ['vector','fts']) await this.q(`LOAD ${extension}`); }
  async init(f) {
    await this.open();
    for(const sql of [
      'CREATE NODE TABLE Chunk(id STRING PRIMARY KEY,run_id STRING,text STRING,lexical STRING,vector FLOAT[1024])',
      'CREATE NODE TABLE Entity(id STRING PRIMARY KEY,name STRING,type STRING,description STRING,vector FLOAT[1024])',
      'CREATE REL TABLE Mentions(FROM Chunk TO Entity)',
      'CREATE REL TABLE Related(FROM Entity TO Entity,id STRING,relation STRING,evidence STRING,run_ids STRING)',
    ]) await this.q(sql);
    await this.q('BEGIN TRANSACTION');
    for(const c of f.chunks) await this.addChunk(c);
    for(const e of f.entities) await this.addEntity(e);
    for(const [c,e] of f.mentions) await this.q(`MATCH (c:Chunk {id:${s(c)}}),(e:Entity {id:${s(e)}}) CREATE (c)-[:Mentions]->(e)`);
    for(const e of f.edges) await this.q(`MATCH (a:Entity {id:${s(e.source)}}),(b:Entity {id:${s(e.target)}}) CREATE (a)-[:Related {id:${s(e.id)},relation:${s(e.relation)},evidence:${s(e.evidence)},run_ids:${s(JSON.stringify(e.runIds))}}]->(b)`);
    await this.q('COMMIT');
    await this.buildIndexes();
  }
  addChunk(c) {
    return this.q(`CREATE (:Chunk {id:${s(c.id)},run_id:${s(c.runId)},text:${s(c.text)},lexical:${s(tokens(c.text).join(' '))},vector:${s(c.vector)}})`);
  }
  addEntity(e) {
    return this.q(`CREATE (:Entity {id:${s(e.id)},name:${s(e.name)},type:${s(e.type)},description:${s(e.description)},vector:${s(e.vector)}})`);
  }
  async buildIndexes() {
    for(const table of ['Chunk','Entity']) await this.q(`CALL CREATE_VECTOR_INDEX('${table}','vec','vector',metric:='cosine')`);
    await this.q("CALL CREATE_FTS_INDEX('Chunk','fts',['lexical'],stemmer:='none')");
  }
  semantic(v,table='chunk') {
    const label=table==='chunk'?'Chunk':'Entity';
    return this.q(`CALL QUERY_VECTOR_INDEX('${label}','vec',${s(v)},8,efs:=128) RETURN node.id,distance ORDER BY distance,node.id`);
  }
  lexical(query) {
    const terms=[...new Set(tokens(query))].join(' ');
    if(!terms) return [];
    return this.q(`CALL QUERY_FTS_INDEX('Chunk','fts',${s(terms)}) RETURN node.id,score ORDER BY score DESC,node.id LIMIT 8`);
  }
  async graph(seeds) {
    const ids=s(seeds);
    const rows=await this.q(`MATCH (c:Chunk)-[:Mentions]->(e:Entity) WHERE e.id IN ${ids} RETURN DISTINCT c.run_id,e.id`);
    const neighbors=await this.q(`MATCH (seed:Entity)-[:Related]-(e:Entity)<-[:Mentions]-(c:Chunk) WHERE seed.id IN ${ids} RETURN DISTINCT c.run_id,e.id`);
    return [...rows,...neighbors];
  }
  async export() {
    return {nodes:await this.q('MATCH (e:Entity) RETURN e.id,e.name,e.type,e.description ORDER BY e.id'),
      edges:await this.q('MATCH (a:Entity)-[r:Related]->(b:Entity) RETURN r.id,a.id,b.id,r.relation,r.evidence,r.run_ids ORDER BY r.id'),
      mentions:await this.q('MATCH (c:Chunk)-[:Mentions]->(e:Entity) RETURN c.run_id,e.id')};
  }
  async control(f,revision) {
    const next=revision==='after';
    await this.q('BEGIN TRANSACTION');
    if(next) {
      await this.q('MATCH (c:Chunk {id:"native-control"}) SET c.text="codepetnewfixture9ad2",c.lexical="codepetnewfixture9ad2",c.vector='+s(f.chunks[1].vector));
      await this.q('MATCH (c:Chunk {id:"native-control"})-[m:Mentions]->() DELETE m');
      await this.q('MATCH (e:Entity {id:"native-old"}) DETACH DELETE e');
    } else {
      await this.addChunk({id:'native-control',runId:'native-control',text:'codepetoldfixture7fe4',vector:f.chunks[0].vector});
      await this.addChunk({id:'native-keeper',runId:'native-keeper',text:'Shared entity keeper',vector:f.chunks[0].vector});
      await this.addEntity({id:'native-shared',name:'Shared',type:'Control',description:'',vector:f.chunks[0].vector});
      await this.q('MATCH (c:Chunk {id:"native-keeper"}),(e:Entity {id:"native-shared"}) CREATE (c)-[:Mentions]->(e)');
    }
    const id=next?'native-new':'native-old';
    await this.addEntity({id,name:next?'New':'Old',type:'Control',description:'',vector:f.chunks[0].vector});
    for(const entity of [id,'native-shared']) await this.q(`MATCH (c:Chunk {id:'native-control'}),(e:Entity {id:${s(entity)}}) CREATE (c)-[:Mentions]->(e)`);
    await this.q(`MATCH (a:Entity {id:${s(id)}}),(b:Entity {id:'native-shared'}) CREATE (a)-[:Related {id:'native-edge',relation:'uses',evidence:'synthetic control',run_ids:'[]'}]->(b)`);
    await this.q('COMMIT');
  }
  async cleanup() {
    await this.q('BEGIN TRANSACTION');
    await this.q("MATCH (c:Chunk) WHERE c.id IN ['native-control','native-keeper'] DETACH DELETE c");
    await this.q("MATCH (e:Entity) WHERE e.id IN ['native-new','native-shared'] DETACH DELETE e");
    await this.q('COMMIT');
  }
  close() { return this.db.close(); }
}
