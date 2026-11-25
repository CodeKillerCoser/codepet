import { NativeConnection, literal } from '../ipc.mjs';
import { validateSnapshot, refreshNames } from './snapshot.mjs';

export class GraphStore {
  constructor(driver, database, audit=[]) { this.connection=new NativeConnection(driver,database,audit); }
  q(sql) { return this.connection.query(sql); }
  async initialize(snapshot) {
    validateSnapshot(snapshot);
    await this.q('CREATE NODE TABLE GraphNode(id STRING PRIMARY KEY,position INT64,payload STRING)');
    await this.q('CREATE REL TABLE GraphLink(FROM GraphNode TO GraphNode,position INT64,relation STRING,payload STRING)');
    await this.q('CREATE NODE TABLE ViewSnapshot(id STRING PRIMARY KEY,payload STRING)');
    await this.q('BEGIN TRANSACTION');
    try {
      for (const [position,node] of snapshot.nodes.entries()) {
        await this.q(`CREATE (:GraphNode {id:${literal(node.id)},position:${position},payload:${literal(JSON.stringify(node))}})`);
      }
      for (const [position,edge] of snapshot.links.entries()) {
        await this.q(`MATCH (a:GraphNode {id:${literal(edge.source)}}),(b:GraphNode {id:${literal(edge.target)}})
          CREATE (a)-[:GraphLink {position:${position},relation:${literal(edge.relation)},payload:${literal(JSON.stringify(edge))}}]->(b)`);
      }
      const {nodes,links,...metadata}=snapshot;
      await this.q(`CREATE (:ViewSnapshot {id:'cognee-1.6.10',payload:${literal(JSON.stringify(metadata))}})`);
      await this.q('COMMIT');
    } catch(error) { await this.q('ROLLBACK');throw error; }
  }
  async read() {
    const nodes=(await this.q('MATCH (n:GraphNode) RETURN n.payload ORDER BY n.position')).map(r=>JSON.parse(r[0]));
    const links=(await this.q('MATCH ()-[r:GraphLink]->() RETURN r.payload ORDER BY r.position')).map(r=>JSON.parse(r[0]));
    const rows=await this.q("MATCH (m:ViewSnapshot {id:'cognee-1.6.10'}) RETURN m.payload");
    const snapshot={...JSON.parse(rows[0][0]),nodes,links};
    validateSnapshot(snapshot);
    return refreshNames(snapshot);
  }
  async replaceNode(node) {
    await this.q(`MATCH (n:GraphNode {id:${literal(node.id)}}) SET n.payload=${literal(JSON.stringify(node))}`);
  }
  close() { return this.connection.close(); }
}
