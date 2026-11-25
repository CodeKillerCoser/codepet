import { readFile, writeFile, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { sha256 } from './snapshot.mjs';

const exec=promisify(execFile);
async function download(url,path) {
  try {await stat(path);}catch {
    await exec('curl',['-fsSL','--retry','2','--connect-timeout','15','--max-time','60',url,'-o',path]);
  }
}
export async function stageFonts(root) {
  const url='https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap';
  const path=join(root,'assets/inter-upstream.css');await download(url,path);
  let css=await readFile(path,'utf8');
  const files=[];
  for(const source of [...new Set([...css.matchAll(/url\((https:\/\/[^)]+)\)/g)].map(m=>m[1]))]) {
    const name='inter-'+sha256(source).slice(0,14)+'.'+source.split('.').at(-1);
    await download(source,join(root,'assets',name));
    const data=await readFile(join(root,'assets',name));
    files.push({name,url:source,bytes:data.length,sha256:sha256(data)});
    css=css.replaceAll(source,'/assets/'+name);
  }
  await writeFile(join(root,'assets/inter.css'),css);
  await download('https://raw.githubusercontent.com/rsms/inter/master/LICENSE.txt',join(root,'licenses/inter-LICENSE.txt'));
  return {cssURL:url,cssSHA256:sha256(css),files};
}
