import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { safeJSON, validateSnapshot } from './snapshot.mjs';

const chunks={UI_CHROME:'ui_chrome',SCHEMA_VIEW:'schema_view',STORY_VIEW:'story_view',
  INSPECTOR:'inspector',MEMORY_VIEW:'memory_map',SEMANTIC_VIEW:'semantic_map'};

export async function render(snapshot, assets) {
  validateSnapshot(snapshot);
  let html=await readFile(join(assets,'template.html'),'utf8');
  for(const [token,name] of Object.entries(chunks)) {
    const source=await readFile(join(assets,name+'.js'),'utf8');
    html=html.replaceAll(`__${token}_JS__`,()=>source);
  }
  html=html.replaceAll('__PIPELINE_LAYOUT_JS__','')
    .replaceAll('__SEMANTIC_LAYOUT_JS__','window._semanticPositions = __SEMANTIC_POSITIONS__;');
  const values={NODES_DATA:snapshot.nodes,LINKS_DATA:snapshot.links,TASK_COLORS:snapshot.taskColors,
    PIPELINE_COLORS:snapshot.pipelineColors,NODESET_COLORS:snapshot.nodesetColors,USER_COLORS:snapshot.userColors,
    SCHEMA_DATA:snapshot.schemaData,SCHEMA_GRAPH_DATA:snapshot.schemaGraph,MEMORY_DATA:snapshot.memory,
    SEARCH_EVENTS:snapshot.searchEvents,SEMANTIC_POSITIONS:snapshot.semanticPositions,SEMANTIC_CLUSTERS:snapshot.semanticClusters};
  for(const [token,value] of Object.entries(values)) html=html.replaceAll(`__${token}__`,()=>safeJSON(value));
  if(/__[A-Z_]+__/.test(html)) throw new Error('Unresolved renderer tokens');
  // Keep Cognee view code unchanged; resolve its one network dependency locally.
  html=html.replace('https://d3js.org/d3.v7.min.js','/assets/d3.v7.9.0.min.js');
  html=html.replace('https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap','/assets/inter.css')
    .replace(/<link[^>]*rel="preconnect"[^>]*>/g,'');
  return html;
}
