export const viewFiles = {
  UI_CHROME: 'ui_chrome', SCHEMA_VIEW: 'schema_view', STORY_VIEW: 'story_view',
  INSPECTOR: 'inspector', MEMORY_VIEW: 'memory_map', SEMANTIC_VIEW: 'semantic_map',
};

export function normalizeSnapshot(input) {
  if (!input || !Array.isArray(input.nodes) || !Array.isArray(input.links)) {
    throw new TypeError('A visualization snapshot with nodes and links is required');
  }
  if (!input.nodes.length) throw new TypeError('A nonempty node snapshot is required; use destroy() to clear the view');
  const ids = new Set(input.nodes.map(node => node.id));
  if (ids.size !== input.nodes.length || [...ids].some(id => typeof id !== 'string' || !id)) {
    throw new TypeError('Node IDs must be unique nonempty strings');
  }
  for (const link of input.links) {
    if (!ids.has(link.source) || !ids.has(link.target)) throw new TypeError('Dangling graph link');
  }
  return {
    nodes: input.nodes, links: input.links,
    taskColors: input.taskColors ?? input.color_maps?.task ?? {},
    pipelineColors: input.pipelineColors ?? input.color_maps?.pipeline ?? {},
    nodesetColors: input.nodesetColors ?? input.color_maps?.node_set ?? {},
    userColors: input.userColors ?? input.color_maps?.user ?? {},
    schemaData: input.schemaData ?? input.schema_data ?? null,
    schemaGraph: input.schemaGraph ?? input.schema_graph ?? { nodes: [], links: [] },
    memory: input.memory ?? input.memory_map ?? {},
    searchEvents: input.searchEvents ?? input.search_events ?? [],
    semanticPositions: input.semanticPositions ?? input.semantic_positions ?? null,
    semanticClusters: input.semanticClusters ?? input.semantic_clusters ?? null,
  };
}

function safeJSON(value) {
  return JSON.stringify(value).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
}

// Browser-compatible pure assembly. No filesystem, database, process or model API.
export function assemble(input, resources) {
  const snapshot = normalizeSnapshot(input);
  let html = resources.template;
  for (const [token, file] of Object.entries(viewFiles)) {
    if (typeof resources[file] !== 'string') throw new TypeError(`Missing view: ${file}`);
    html = html.replaceAll(`__${token}_JS__`, () => resources[file]);
  }
  html = html.replaceAll('__PIPELINE_LAYOUT_JS__', '')
    .replaceAll('__SEMANTIC_LAYOUT_JS__', 'window._semanticPositions = __SEMANTIC_POSITIONS__;');
  const values = {
    NODES_DATA: snapshot.nodes, LINKS_DATA: snapshot.links, TASK_COLORS: snapshot.taskColors,
    PIPELINE_COLORS: snapshot.pipelineColors, NODESET_COLORS: snapshot.nodesetColors,
    USER_COLORS: snapshot.userColors, SCHEMA_DATA: snapshot.schemaData,
    SCHEMA_GRAPH_DATA: snapshot.schemaGraph, MEMORY_DATA: snapshot.memory,
    SEARCH_EVENTS: snapshot.searchEvents, SEMANTIC_POSITIONS: snapshot.semanticPositions,
    SEMANTIC_CLUSTERS: snapshot.semanticClusters,
  };
  for (const [token, value] of Object.entries(values)) html = html.replaceAll(`__${token}__`, () => safeJSON(value));
  if (/__[A-Z_]+__/.test(html)) throw new Error('Unresolved Cognee template token');
  html = html.replace(/<script src="https:\/\/d3js\.org\/d3\.v7\.min\.js"><\/script>/,
    () => `<script>${resources.d3}</script>`);
  html = html.replace(/<link href="https:\/\/fonts\.googleapis\.com[^"\n]+" rel="stylesheet">/,
    () => `<style>${resources.fontCSS}</style>`);
  html = html.replace(/<link[^>]*rel="preconnect"[^>]*>/g, '');
  html = html.replace('</head>',() => `<style>${resources.embedCSS ?? ''}</style></head>`);
  // The original renderer runs offline in an opaque sandbox, without Host IPC.
  return html.replace('<head>', `<head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'">`);
}
