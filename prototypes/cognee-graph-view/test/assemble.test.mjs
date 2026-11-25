import { test } from 'node:test';
import assert from 'node:assert/strict';
import { assemble, normalizeSnapshot, viewFiles } from '../src/assemble.mjs';

const nodes = [{ id: 'a', name: 'real node' }];
test('accepts the Cognee JSON protocol without a database or Node renderer', () => {
  const input = { nodes, links: [], color_maps: {task:{source:'#aaa'}},
    schema_graph:{nodes:[{id:'type:Entity'}],links:[]}, memory_map:{documents:[]},
    semantic_positions:{a:{x:0,y:0}}, semantic_clusters:{clusters:[]} };
  const before = JSON.stringify(input);
  const normalized = normalizeSnapshot(input);
  assert.equal(normalized.taskColors.source,'#aaa');
  assert.equal(normalized.schemaGraph.nodes[0].id,'type:Entity');
  assert.equal(normalized.semanticPositions.a.x,0);
  assert.equal(JSON.stringify(input),before);
});
test('rejects ambiguous IDs and broken relationship references before touching a view', () => {
  assert.throws(()=>normalizeSnapshot({nodes:[],links:[]}),/nonempty/);
  assert.throws(()=>normalizeSnapshot({nodes:[...nodes,...nodes],links:[]}),/unique/);
  assert.throws(()=>normalizeSnapshot({nodes,links:[{source:'a',target:'missing'}]}),/Dangling/);
});
test('embeds untrusted graph text as JSON while supporting a nested static asset path', () => {
  const attack = '</script><script>window.UNSAFE=true</script>';
  const input = {nodes:[{id:'a',name:attack}],links:[]};
  const resources = {template:'<script src="https://d3js.org/d3.v7.min.js"></script>\n<script>var nodes=__NODES_DATA__;__UI_CHROME_JS__</script>'};
  for(const file of Object.values(viewFiles))resources[file]='';
  const html = assemble(input,resources,'https://example.test/nested/widget/assets/');
  assert(!html.includes(attack));
  assert(html.includes('https://example.test/nested/widget/assets/d3.v7.9.0.min.js'));
  assert.equal([...html.matchAll(/<script[^>]+src="([^"]+)"/g)][0][1],
    'https://example.test/nested/widget/assets/d3.v7.9.0.min.js');
  assert(html.includes('\\u003c/script>'));
  assert(!/__NODES_DATA__/.test(html));
});
test('localizes already adapted asset URLs once without doubling the host prefix', () => {
  const resources={template:'<script src="/assets/d3.v7.9.0.min.js"></script><link href="/assets/inter.css">'};
  for(const file of Object.values(viewFiles))resources[file]='';
  const html=assemble({nodes,links:[]},resources,'https://example.test/nested/assets/');
  assert(html.includes('src="https://example.test/nested/assets/d3.v7.9.0.min.js"'));
  assert(html.includes('href="https://example.test/nested/assets/inter.css"'));
});
test('public entry can be imported during SSR before a browser DOM exists', async () => {
  const api = await import('../src/index.mjs');
  assert.equal(typeof api.mountCogneeGraph,'function');
});
