// Development-only synthetic stress data. Never reads or writes a task ledger.
import {createServer} from 'vite';
import {readFileSync} from 'node:fs';
const port=Number(process.env.CODEPET_GRAPH_PROBE_PORT||1428);
function graph(count){
 const kinds=['TextDocument','DocumentChunk','Entity','EntityType','TextSummary'];
 const nodes=Array.from({length:count},(_,i)=>({id:`stress-${i}`,name:`Synthetic ${kinds[i%5]} ${i}`,type:kinds[i%5],
  stage:['document','chunk','entity','type','summary'][i%5],visual_rank:Math.floor(i/5),topological_rank:i%5,degree:6,importance:1,
  source_task:`task-${Math.floor(i/250)}`,source_pipeline:'stress',source_user:'synthetic',color:'#8164df',
  text:'Synthetic load-test node; no user task data.'}));
 const links=nodes.flatMap((n,i)=>[1,29,811].map((step,k)=>({source:n.id,target:`stress-${(i+step)%count}`,
  relation:k?'related_to':'contains',edge_class:k?'semantic':'structural'})));
 return {nodes,links:count===700?links.slice(0,1600):links,schema_graph:{nodes:[],links:[]},memory_map:{},color_maps:{task:{},pipeline:{},node_set:{},user:{}},search_events:[]};
}
// Inject metrics only in this development server's iframe response.
function frameMetrics(){
 const started=performance.now(),gaps=[],longTasks=[];let last=started;
 if(window.PerformanceObserver){try{new PerformanceObserver(list=>longTasks.push(...list.getEntries().map(e=>e.duration))).observe({type:'longtask',buffered:true});}catch{}}
 function frame(now){gaps.push(now-last);last=now;if(now-started<2200)requestAnimationFrame(frame);else{
  const sorted=gaps.slice().sort((a,b)=>a-b);
  parent.postMessage({type:'stress:metrics',nodes:Object.keys(window._vizNodeById||{}).length,frames:gaps.length,
   fps:gaps.length*1000/(now-started),p95FrameMs:sorted[Math.floor(sorted.length*.95)],maxFrameMs:Math.max(...gaps),
   longTasks:longTasks.length,maxLongTaskMs:longTasks.length?Math.max(...longTasks):0},'*');
 }}
 requestAnimationFrame(frame);
 window.addEventListener('message',e=>{if(e.source!==parent||e.data?.type!=='stress:ping')return;
  const begin=performance.now();window._requestGraphRedraw();parent.postMessage({type:'stress:pong',nonce:e.data.nonce,drawMs:performance.now()-begin},'*');
 });
}
const html=`<!doctype html><meta charset="utf-8"><title>Synthetic graph stress test</title><style>body{margin:0;font:14px system-ui}header{display:flex;align-items:center;gap:12px;padding:8px;height:36px}#graph{height:calc(100vh - 52px)}output{font-size:12px}</style><header><strong>合成压力图 · 非业务数据</strong><select id="count"><option>700</option><option>5000</option><option>20000</option><option>50000</option><option>100000</option></select><button id="load">加载</button><output id="result"></output></header><div id="graph"></div><script type="module">
import {mountCogneeGraph} from '/cognee-view/index.js';
window.samples=[];window.metrics=[];window.errors=[];window.addEventListener('message',e=>{if(e.source===document.querySelector('iframe')?.contentWindow&&e.data?.type==='stress:metrics')window.metrics.push(e.data);});window.ping=()=>new Promise(resolve=>{const start=performance.now(),nonce=crypto.randomUUID();const receive=e=>{if(e.source===document.querySelector('iframe')?.contentWindow&&e.data?.type==='stress:pong'&&e.data.nonce===nonce){window.removeEventListener('message',receive);resolve({roundTripMs:performance.now()-start,drawMs:e.data.drawMs});}};window.addEventListener('message',receive);document.querySelector('iframe').contentWindow.postMessage({type:'stress:ping',nonce},'*');});window.addEventListener('error',e=>window.errors.push(e.message));let view;
window.start=async count=>{view?.destroy();view=null;window.done=false;const sample={nodes:count};const t=performance.now();document.querySelector('output').textContent='加载中…';
 try{const response=await fetch('/__stress/graph?count='+count);const raw=await response.text();sample.bytes=new TextEncoder().encode(raw).length;const parse=performance.now();const data=JSON.parse(raw);sample.fetchMs=parse-t;sample.parseMs=performance.now()-parse;sample.edges=data.links.length;const mount=performance.now();view=await mountCogneeGraph(document.querySelector('#graph'),data);sample.mountMs=performance.now()-mount;sample.totalMs=performance.now()-t;sample.ok=true;}catch(e){sample.ok=false;sample.error=String(e);sample.totalMs=performance.now()-t;}
 window.samples.push(sample);document.querySelector('output').textContent=JSON.stringify(sample);window.done=true;return sample;};
 document.querySelector('#load').onclick=()=>window.start(Number(document.querySelector('#count').value));window.probeReady=true;
</script>`;
const server=await createServer({server:{host:'127.0.0.1',port,strictPort:true},plugins:[{name:'synthetic-stress-only',configureServer(s){s.middlewares.use((req,res,next)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname==='/cognee-view/frame.js'){res.setHeader('Content-Type','text/javascript');res.end(readFileSync(new URL('../public/cognee-view/frame.js',import.meta.url),'utf8').replace('const initialized = window.d3','('+frameMetrics.toString()+')(); const initialized = window.d3'));}
 else if(url.pathname==='/__stress/'){res.setHeader('Content-Type','text/html');res.end(html);}
 else if(url.pathname==='/__stress/graph'){const count=Number(url.searchParams.get('count'));if(![700,5000,20000,50000,100000].includes(count)){res.statusCode=400;res.end();return;}res.setHeader('Content-Type','application/json');res.end(JSON.stringify(graph(count)));}
 else next();
 });}}]});await server.listen();console.log(`Synthetic graph probe: http://127.0.0.1:${port}/__stress/`);
