import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';

const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/native-comparison');
const allowed=new Map([['/','index.html'],['/index.html','index.html'],['/view.json','view.json'],
  ['/comparison.json','comparison.json'],['/documents.json','documents.json']]);
const server=createServer(async(request,response)=>{
  const path=allowed.get(new URL(request.url,'http://127.0.0.1').pathname);
  if(!path||request.method!=='GET'){response.writeHead(404);response.end();return;}
  try {
    const body=await readFile(join(root,path));
    response.writeHead(200,{'Content-Type':path.endsWith('.html')?'text/html; charset=utf-8':'application/json; charset=utf-8',
      'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'});
    response.end(body);
  }catch{response.writeHead(404);response.end();}
});
server.listen(Number(process.argv[3]||50546),'127.0.0.1',()=>console.log(`http://127.0.0.1:${server.address().port}/`));
