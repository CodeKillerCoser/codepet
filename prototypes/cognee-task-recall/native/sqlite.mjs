import { NativeConnection, tokens, literal } from './ipc.mjs';

export class SQLite {
  constructor(root, path, audit) { this.db = new NativeConnection(`${root}/sqlite-driver`, path, audit); }
  q(sql, params) { return this.db.query(sql, params); }
  async init(f) {
    for (const sql of [
      'PRAGMA foreign_keys=ON',
      'CREATE TABLE chunk(id TEXT PRIMARY KEY, seq INTEGER UNIQUE, run_id TEXT, text TEXT)',
      'CREATE TABLE entity(id TEXT PRIMARY KEY,name TEXT,type TEXT,description TEXT,seq INTEGER UNIQUE)',
      'CREATE TABLE mentions(chunk TEXT REFERENCES chunk(id),entity TEXT REFERENCES entity(id),PRIMARY KEY(chunk,entity))',
      'CREATE TABLE relation(id TEXT PRIMARY KEY,source TEXT REFERENCES entity(id),target TEXT REFERENCES entity(id),relation TEXT,evidence TEXT,run_ids TEXT)',
      'CREATE INDEX relation_source ON relation(source)', 'CREATE INDEX relation_target ON relation(target)',
      'CREATE INDEX mentions_entity ON mentions(entity)',
      'CREATE VIRTUAL TABLE chunk_fts USING fts5(text,chunk_id UNINDEXED)',
      'CREATE VIRTUAL TABLE chunk_vec USING vec0(id INTEGER PRIMARY KEY,embedding FLOAT[1024] distance_metric=cosine)',
      'CREATE VIRTUAL TABLE entity_vec USING vec0(id INTEGER PRIMARY KEY,embedding FLOAT[1024] distance_metric=cosine)',
    ]) await this.q(sql);
    await this.q('BEGIN');
    for (const [i, c] of f.chunks.entries()) await this.addChunk(c, i + 1);
    for (const [i, e] of f.entities.entries()) await this.addEntity(e, i + 1);
    for (const [c, e] of f.mentions) await this.q('INSERT INTO mentions VALUES(?,?)', [c, e]);
    for (const e of f.edges) await this.q('INSERT INTO relation VALUES(?,?,?,?,?,?)', [e.id,e.source,e.target,e.relation,e.evidence,JSON.stringify(e.runIds)]);
    await this.q('COMMIT');
  }
  async addChunk(c, seq) {
    await this.q('INSERT INTO chunk VALUES(?,?,?,?)', [c.id,seq,c.runId,c.text]);
    await this.q('INSERT INTO chunk_fts(text,chunk_id) VALUES(?,?)', [tokens(c.text).join(' '),c.id]);
    await this.q('INSERT INTO chunk_vec VALUES(?,?)', [seq,c.vector]);
  }
  async addEntity(e, seq) {
    await this.q('INSERT INTO entity VALUES(?,?,?,?,?)', [e.id,e.name,e.type,e.description,seq]);
    await this.q('INSERT INTO entity_vec VALUES(?,?)', [seq,e.vector]);
  }
  semantic(v, table='chunk') {
    return this.q(`SELECT x.id,v.distance FROM ${table}_vec v JOIN ${table} x ON x.seq=v.id WHERE embedding MATCH ? AND k=8 ORDER BY distance,x.id`, [v]);
  }
  lexical(query) {
    const terms = [...new Set(tokens(query))].map(t => literal(t)).join(' OR ');
    if (!terms) return [];
    return this.q('SELECT chunk_id,bm25(chunk_fts) FROM chunk_fts WHERE chunk_fts MATCH ? ORDER BY bm25(chunk_fts),chunk_id LIMIT 8', [terms]);
  }
  graph(seeds) {
    return this.q(`WITH seed(id) AS (VALUES ${seeds.map(()=>'(?)').join(',')}),
      reach(id) AS (SELECT id FROM seed UNION SELECT target FROM relation WHERE source IN (SELECT id FROM seed)
      UNION SELECT source FROM relation WHERE target IN (SELECT id FROM seed))
      SELECT DISTINCT c.run_id,m.entity FROM reach r JOIN mentions m ON m.entity=r.id JOIN chunk c ON c.id=m.chunk`, seeds);
  }
  async export() {
    return { nodes: await this.q('SELECT id,name,type,description FROM entity ORDER BY id'),
      edges: await this.q('SELECT id,source,target,relation,evidence,run_ids FROM relation ORDER BY id'),
      mentions: await this.q('SELECT c.run_id,m.entity FROM mentions m JOIN chunk c ON c.id=m.chunk') };
  }
  async control(f, revision) {
    const id = 'native-control';
    await this.q('BEGIN');
    if (revision !== 'before') {
      await this.q('DELETE FROM mentions WHERE chunk=?',[id]);
      await this.q('DELETE FROM chunk_vec WHERE id=10001');
      await this.q('DELETE FROM chunk_fts WHERE chunk_id=?',[id]);
      await this.q('DELETE FROM chunk WHERE id=?',[id]);
      await this.q('DELETE FROM relation WHERE id=?',['native-edge']);
      await this.q('DELETE FROM entity_vec WHERE id=10001');
      await this.q('DELETE FROM entity WHERE id=?',['native-old']);
    }
    if (revision === 'before') {
      await this.addEntity({id:'native-shared',name:'Shared',type:'Control',description:'',vector:f.chunks[0].vector},10002);
      await this.addChunk({id:'native-keeper',runId:'native-keeper',text:'Shared entity keeper',vector:f.chunks[0].vector},10002);
      await this.q('INSERT INTO mentions VALUES(?,?)',['native-keeper','native-shared']);
    }
    if (revision !== 'remove') {
      const next = revision === 'after';
      await this.addChunk({id,runId:id,text:next?'codepetnewfixture9ad2':'codepetoldfixture7fe4',vector:f.chunks[next?1:0].vector},10001);
      await this.addEntity({id:next?'native-new':'native-old',name:next?'New':'Old',type:'Control',description:'',vector:f.chunks[0].vector},10001);
      for (const entity of [next?'native-new':'native-old','native-shared']) await this.q('INSERT INTO mentions VALUES(?,?)',[id,entity]);
      await this.q('INSERT INTO relation VALUES(?,?,?,?,?,?)',['native-edge',next?'native-new':'native-old','native-shared','uses','synthetic control','[]']);
    }
    await this.q('COMMIT');
  }
  async cleanup() {
    // After control('after'), remove both synthetic documents and their referenced graph records.
    for (const sql of ['BEGIN',"DELETE FROM mentions WHERE chunk IN ('native-control','native-keeper')",
      "DELETE FROM relation WHERE id='native-edge'",'DELETE FROM chunk_vec WHERE id>=10001',
      "DELETE FROM chunk_fts WHERE chunk_id IN ('native-control','native-keeper')",
      "DELETE FROM chunk WHERE id IN ('native-control','native-keeper')",'DELETE FROM entity_vec WHERE id>=10001',
      "DELETE FROM entity WHERE id IN ('native-old','native-new','native-shared')",'COMMIT']) await this.q(sql);
  }
  close() { return this.db.close(); }
}
