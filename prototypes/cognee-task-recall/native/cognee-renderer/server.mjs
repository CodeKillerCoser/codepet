import { createServer } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { GraphStore } from './store.mjs';
import { render } from './render.mjs';

const root=resolve(process.argv[2]||join(dirname(fileURLToPath(import.meta.url)),'..'));
const port=Number(process.argv[3]||50547), audit=[];
const store=new GraphStore(join(root,'native/ladybug-driver'),join(root,'graph.lbdb'),audit);
const assets=join(root,'assets');
const provenance=JSON.parse(await readFile(join(root,'provenance.json'),'utf8'));
const publicAssets=new Set(['d3.v7.9.0.min.js','inter.css',...(provenance.fonts?.files||[]).map(f=>f.name)]);
const server=createServer(async(request,response)=>{
  try {
    const path=new URL(request.url,'http://127.0.0.1').pathname;
    if(request.method!=='GET'){response.writeHead(405);response.end();return;}
    let body,type;
    if(path==='/'||path==='/index.html') {
      body=await render(await store.read(),assets);type='text/html; charset=utf-8';
    } else if(path==='/graph.json') {
      body=JSON.stringify(await store.read());type='application/json; charset=utf-8';
    } else if(path==='/provenance.json') {
      body=await readFile(join(root,'provenance.json'));type='application/json; charset=utf-8';
    } else if(path.startsWith('/assets/')&&publicAssets.has(path.slice(8))) {
      body=await readFile(join(assets,path.slice(8)));
      type=path.endsWith('.css')?'text/css; charset=utf-8':path.endsWith('.ttf')?'font/ttf':path.endsWith('.woff2')?'font/woff2':'application/javascript; charset=utf-8';
    } else {response.writeHead(404);response.end();return;}
    response.writeHead(200,{'Content-Type':type,'Cache-Control':'no-store',
      'X-Content-Type-Options':'nosniff',
      'Content-Security-Policy':"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'"});
    response.end(body);
    await writeFile(join(root,'runtime-audit.json'),JSON.stringify({pid:process.pid,
      executable:process.execPath,childExecutable:join(root,'native/ladybug-driver'),
      pythonInvoked:false,readQueries:audit.length,lastReadAt:new Date().toISOString()},null,2));
  } catch(error) {response.writeHead(500);response.end(String(error));}
});
server.listen(port,'127.0.0.1',()=>console.log(JSON.stringify({url:`http://127.0.0.1:${server.address().port}/`,
  runtime:'Node + native Ladybug C++ library; no Python',root})));
async function shutdown(){server.close();await store.close();process.exit(0);}
process.on('SIGTERM',shutdown);process.on('SIGINT',shutdown);
