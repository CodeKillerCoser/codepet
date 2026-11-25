import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

// Read only a JSON literal at a known renderer assignment, never evaluate upstream JavaScript.
export function readAssignment(source, marker) {
  const index = source.indexOf(marker);
  assert(index >= 0, `Missing renderer assignment: ${marker}`);
  let start = index + marker.length;
  while (/\s/.test(source[start] || '')) start++;
  if (source.slice(start, start + 4) === 'null') return null;
  assert(['[', '{'].includes(source[start]), `Expected JSON at ${marker}`);
  let depth = 0, quoted = false, escaped = false;
  for (let i = start; i < source.length; i++) {
    const c = source[i];
    if (quoted) {
      if (escaped) escaped = false;
      else if (c === '\\') escaped = true;
      else if (c === '"') quoted = false;
    } else if (c === '"') quoted = true;
    else if (c === '{' || c === '[') depth++;
    else if (c === '}' || c === ']') {
      if (--depth === 0) return JSON.parse(source.slice(start, i + 1));
    }
  }
  throw new Error(`Unterminated renderer assignment: ${marker}`);
}

export const sha256 = value => createHash('sha256').update(value).digest('hex');
export function topologyHash(nodes, links) {
  return sha256(JSON.stringify({nodes:nodes.map(n=>n.id).sort(),
    links:links.map(l=>[l.source,l.target,l.relation]).sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)))}));
}

export function extractSnapshot(html) {
  const snapshot = {
    nodes:readAssignment(html, 'var nodes = '),
    links:readAssignment(html, 'var links = '),
    taskColors:readAssignment(html, 'var taskColors = '),
    pipelineColors:readAssignment(html, 'var pipelineColors = '),
    nodesetColors:readAssignment(html, 'var nodesetColors = '),
    userColors:readAssignment(html, 'var userColors = '),
    schemaData:readAssignment(html, 'const schemaData = '),
    schemaGraph:readAssignment(html, 'const schemaGraphData = '),
    memory:readAssignment(html, 'const memoryMap = '),
    searchEvents:readAssignment(html, 'const searchEvents = '),
    semanticPositions:readAssignment(html, 'window._semanticPositions = '),
    semanticClusters:readAssignment(html, 'const CLUSTERS = '),
  };
  snapshot.topologyHash = topologyHash(snapshot.nodes, snapshot.links);
  return snapshot;
}

export function validateSnapshot(snapshot) {
  const ids = new Set(snapshot.nodes.map(n=>n.id));
  assert.equal(ids.size, snapshot.nodes.length, 'duplicate node IDs');
  for (const edge of snapshot.links) {
    assert(ids.has(edge.source)&&ids.has(edge.target), 'dangling graph edge');
  }
  assert.equal(topologyHash(snapshot.nodes,snapshot.links), snapshot.topologyHash,
    'Topology changed: regenerate the processed view snapshot before publishing');
  for (const id of Object.keys(snapshot.semanticPositions || {})) assert(ids.has(id), 'orphan semantic position');
  for (const item of snapshot.memory.documents || []) {
    assert(ids.has(item.id));
    for (const chunk of item.chunks) assert(ids.has(chunk.id));
  }
}

export function refreshNames(snapshot) {
  // Structural presentation metadata is retained from the processed snapshot.
  // A property-only update is resolved from the newly read native node records.
  const map = new Map(snapshot.nodes.map(n=>[n.id,n]));
  for (const [id,record] of Object.entries(snapshot.schemaGraph.instance_index || {})) {
    if (map.has(id)) record.name = map.get(id).name;
  }
  for (const doc of snapshot.memory.documents || []) if (map.has(doc.id)) doc.name=map.get(doc.id).name;
  for (const group of snapshot.memory.entity_groups || []) if (map.has(group.type_id)) group.type_name=map.get(group.type_id).name;
  return snapshot;
}

export function safeJSON(value) {
  return JSON.stringify(value).replace(/</g,'\\u003c').replace(/\u2028/g,'\\u2028').replace(/\u2029/g,'\\u2029');
}
