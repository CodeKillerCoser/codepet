import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { assemble, normalizeSnapshot, viewFiles } from '../public/cognee-view/assemble.js';

test('vendored renderer files match recorded upstream hashes',async () => {
  const root=new URL('../public/cognee-view/vendor/',import.meta.url);
  const info=JSON.parse(await readFile(new URL('provenance.json',root),'utf8'));
  assert.equal(info.cognee,'1.6.10');
  for(const file of info.files) {
    assert.equal(createHash('sha256').update(await readFile(new URL(file.path,root))).digest('hex'),file.sha256,file.path);
  }
});
test('graph text stays JSON data and all renderer assets stay inside the sandbox',() => {
  const attack='</script><script>parent.UNSAFE=true</script>';
  const resources={template:'<head><script src="https://d3js.org/d3.v7.min.js"></script><link href="https://fonts.googleapis.com/css2?family=Inter" rel="stylesheet"></head><body><script>const nodes=__NODES_DATA__</script></body>',d3:'/* pinned d3 */',fontCSS:'/* embedded fonts */'};
  for(const name of Object.values(viewFiles))resources[name]='';
  const html=assemble({nodes:[{id:'a',name:attack}],links:[]},resources);
  assert(!html.includes(attack));assert(html.includes('\\u003c/script>'));
  assert(html.includes("connect-src 'none'"));assert(!html.includes('https://'));
  assert(html.includes('/* pinned d3 */'));assert(html.includes('/* embedded fonts */'));
});
test('invalid source relationships cannot silently render as an unrelated graph',() => {
  assert.throws(()=>normalizeSnapshot({nodes:[{id:'a'}],links:[{source:'a',target:'missing'}]}),/Dangling/);
  assert.throws(()=>normalizeSnapshot({nodes:[{id:'a'},{id:'a'}],links:[]}),/unique/);
});
