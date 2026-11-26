import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source=await readFile(new URL('../public/cognee-view/frame.js',import.meta.url),'utf8');
function harness(){
  const handlers=[],messages=[];
  const elements=new Map();
  const context={parent:{postMessage:message=>messages.push(message)},window:{addEventListener:(_,handler)=>handlers.push(handler),removeEventListener:()=>{}},
    document:{getElementById:id=>{if(!elements.has(id))elements.set(id,{style:{display:''},textContent:'',click(){},classList:{add(){}}});return elements.get(id);}}};
  vm.createContext(context);vm.runInContext(source,context);
  return {context,handlers,messages,elements};
}
test('Semantic waits for native data and accepts results only from its bound parent/token',()=>{
  const {context,handlers,messages,elements}=harness();let rendered=0;
  context.window._renderSemanticView=()=>rendered++;
  context.window._codepetSemanticClusters={};
  context.connectSemantic('bound-token');
  const receive=handlers.at(-1);
  context.window._renderSemanticView();context.window._renderSemanticView();
  assert.equal(messages.length,1,'Repeated clicks share one request');assert.equal(rendered,0);
  receive({source:context.parent,data:{type:'codepet:cognee:semantic-result',token:'wrong-token',positions:{}}});
  assert.equal(rendered,0);
  receive({source:context.parent,data:{type:'codepet:cognee:semantic-error',token:'bound-token'}});
  assert.match(elements.get('semantic-status').textContent,/重试/);
  context.window._renderSemanticView();assert.equal(messages.length,2);
  receive({source:context.parent,data:{type:'codepet:cognee:semantic-result',token:'bound-token',positions:{a:{x:1,y:2}},clusters:{clusters:[{id:1}]}}});
  assert.equal(context.window._semanticPositions.a.x,1);
  assert.equal(context.window._codepetSemanticClusters.clusters[0].id,1);
  assert.equal(rendered,1);context.window._renderSemanticView();assert.equal(rendered,2);
  assert.equal(messages.length,2,'Ready coordinates are reused on subsequent clicks');
});
test('frame initialization does not depend on private upstream simulation variables',()=>{
  const {context,handlers,messages}=harness();let html='';
  Object.assign(context.document,{open(){},write:value=>{html=value;},close(){}});
  context.history={replaceState(){}};
  Object.assign(context.window,{d3:{},_vizNodeById:{a:{fx:1,fy:2}},_vizLinks:[],
    _requestGraphRedraw(){},_renderSchemaGraph(){},_renderMemoryView(){},_renderSemanticView(){},_codepetSemanticClusters:{}});
  handlers[0]({source:context.parent,data:{type:'codepet:cognee:render',token:'bound-token',lazySemantic:true,html:'<head></head><body></body>'}});
  const scripts=[...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(m=>m[1]);
  for(const script of scripts)vm.runInContext(script,context);
  assert.equal(messages.at(-1).type,'codepet:cognee:ready');
  assert.equal(messages.at(-1).nodes,1);
  assert.equal(context.window._vizNodeById.a.x,1);
});
