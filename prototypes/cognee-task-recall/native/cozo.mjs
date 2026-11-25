import { NativeConnection, tokens } from './ipc.mjs';

export class Cozo {
  constructor(root, path, audit) { this.db = new NativeConnection(`${root}/cozo-driver`, path, audit); }
  q(sql, params) { return this.db.query(sql, params); }
  async init(f) {
    for (const sql of [
      ':create chunk {id: String => run_id: String, text: String, lexical: String, vector: <F32;1024>}',
      ':create entity {id: String => name: String, type: String, description: String, vector: <F32;1024>}',
      ':create mentions {chunk: String, entity: String}',
      ':create relation {id: String => source: String, target: String, relation: String, evidence: String, run_ids: String}',
      '::index create relation:source {source}', '::index create relation:target {target}',
      '::index create mentions:entity {entity}',
      '::hnsw create chunk:vec {dim:1024,m:16,dtype:F32,fields:[vector],distance:Cosine,ef_construction:64}',
      '::hnsw create entity:vec {dim:1024,m:16,dtype:F32,fields:[vector],distance:Cosine,ef_construction:64}',
      '::fts create chunk:fts {extractor:lexical,tokenizer:Whitespace,filters:[Lowercase]}',
    ]) await this.q(sql);
    await this.q('?[id,run_id,text,lexical,vector] <- $rows :put chunk {id=>run_id,text,lexical,vector}',
      {rows:f.chunks.map(c=>[c.id,c.runId,c.text,tokens(c.text).join(' '),c.vector])});
    await this.q('?[id,name,type,description,vector] <- $rows :put entity {id=>name,type,description,vector}',
      {rows:f.entities.map(e=>[e.id,e.name,e.type,e.description,e.vector])});
    await this.q('?[chunk,entity] <- $rows :put mentions {chunk,entity}',{rows:f.mentions});
    await this.q('?[id,source,target,relation,evidence,run_ids] <- $rows :put relation {id=>source,target,relation,evidence,run_ids}',
      {rows:f.edges.map(e=>[e.id,e.source,e.target,e.relation,e.evidence,JSON.stringify(e.runIds)])});
  }
  semantic(vector, table='chunk') {
    return this.q(`?[id,d] := ~${table}:vec{id | query:vec($vector),k:8,ef:128,bind_distance:d} :order d,id`,{vector});
  }
  lexical(query) {
    const value = [...new Set(tokens(query))].join(' OR ');
    if (!value) return [];
    return this.q('?[id,score] := ~chunk:fts{id | query:$query,k:8,score_kind:"tf_idf",bind_score:score} :order -score,id',{query:value});
  }
  graph(seeds) {
    return this.q(`seed[id] <- $seeds
      reach[id] := seed[id]
      reach[t] := seed[s], *relation{source:s,target:t}
      reach[s] := seed[t], *relation{source:s,target:t}
      ?[run_id,entity] := reach[entity], *mentions{chunk,entity}, *chunk{id:chunk,run_id}`,
    {seeds:seeds.map(s=>[s])});
  }
  async export() {
    return {nodes:await this.q('?[id,name,type,description] := *entity{id,name,type,description} :order id'),
      edges:await this.q('?[id,source,target,relation,evidence,run_ids] := *relation{id,source,target,relation,evidence,run_ids} :order id'),
      mentions:await this.q('?[run_id,entity] := *mentions{chunk,entity},*chunk{id:chunk,run_id}')};
  }
  async control(f, revision) {
    const next=revision==='after', name=next?'native-new':'native-old';
    const statements=[],params={vector:f.chunks[next?1:0].vector,shared:f.chunks[0].vector};
    if(next) statements.push(
      '{?[chunk,entity] := *mentions{chunk,entity},chunk="native-control" :rm mentions {chunk,entity}}',
      '{?[id] <- [["native-old"]] :rm entity {id}}');
    else statements.push(
      '{?[id,run_id,text,lexical,vector] <- [["native-keeper","native-keeper","Shared entity keeper","shared entity keeper",$shared]] :put chunk {id=>run_id,text,lexical,vector}}',
      '{?[id,name,type,description,vector] <- [["native-shared","Shared","Control","",$shared]] :put entity {id=>name,type,description,vector}}',
      '{?[chunk,entity] <- [["native-keeper","native-shared"]] :put mentions {chunk,entity}}');
    statements.push(
      `{?[id,run_id,text,lexical,vector] <- [["native-control","native-control","${next?'codepetnewfixture9ad2':'codepetoldfixture7fe4'}","${next?'codepetnewfixture9ad2':'codepetoldfixture7fe4'}",$vector]] :put chunk {id=>run_id,text,lexical,vector}}`,
      `{?[id,name,type,description,vector] <- [["${name}","${next?'New':'Old'}","Control","",$shared]] :put entity {id=>name,type,description,vector}}`,
      `{?[chunk,entity] <- [["native-control","${name}"],["native-control","native-shared"]] :put mentions {chunk,entity}}`,
      `{?[id,source,target,relation,evidence,run_ids] <- [["native-edge","${name}","native-shared","uses","synthetic control","[]"]] :put relation {id=>source,target,relation,evidence,run_ids}}`);
    await this.q(statements.join('\n'),params);
  }
  async cleanup() {
    await this.q(`
      {?[chunk,entity] := *mentions{chunk,entity},chunk in ["native-control","native-keeper"] :rm mentions {chunk,entity}}
      {?[id] <- [["native-edge"]] :rm relation {id}}
      {?[id] <- [["native-control"],["native-keeper"]] :rm chunk {id}}
      {?[id] <- [["native-new"],["native-shared"]] :rm entity {id}}`);
  }
  close() { return this.db.close(); }
}
