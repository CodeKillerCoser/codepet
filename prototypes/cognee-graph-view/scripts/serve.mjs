// Static development host only. No graph database, renderer assembly or model calls here.
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { resolve, extname } from 'node:path';

const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/cognee-graph-view');
const types={'.html':'text/html; charset=utf-8','.mjs':'application/javascript; charset=utf-8',
  '.js':'application/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.json':'application/json; charset=utf-8','.ttf':'font/ttf'};
const server=createServer(async(req,res)=>{
  try {
    if(req.method!=='GET'){res.writeHead(405);res.end();return;}
    const pathname=decodeURIComponent(new URL(req.url,'http://127.0.0.1').pathname);
    if(pathname==='/'){res.writeHead(302,{Location:'/example/index.html'});res.end();return;}
    const file=resolve(root,'.'+(pathname==='/'?'/example/index.html':pathname));
    if(!file.startsWith(root+'/')){res.writeHead(404);res.end();return;}
    if(!(await stat(file)).isFile()){res.writeHead(404);res.end();return;}
    res.writeHead(200,{'Content-Type':types[extname(file)]||'application/octet-stream','Cache-Control':'no-store'});
    res.end(await readFile(file));
  } catch {res.writeHead(404);res.end();}
});
server.listen(Number(process.argv[3]||50548),'127.0.0.1',()=>console.log(`http://127.0.0.1:${server.address().port}/example/index.html`));
