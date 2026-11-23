import { build } from 'vite';
import { spawn, execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile, unlink } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import assert from 'node:assert/strict';
const root=resolve(import.meta.dirname,'..'), output=join(root,'artifacts/task-ledger-qa/native');await mkdir(output,{recursive:true});
if(!process.argv.includes('--skip-build')){
  await build({root,build:{outDir:join(root,'artifacts/codex-workbench-qa/native/site'),emptyOutDir:true,rollupOptions:{input:join(root,'frontend/qa/codex-workbench-native.html')}}});
  await new Promise((resolveBuild,reject)=>{const child=spawn('cargo',['build','--manifest-path','src-tauri/Cargo.toml','--example','workbench_native_qa','--features','custom-protocol'],{cwd:root,stdio:'inherit',env:{...process.env,TAURI_CONFIG:JSON.stringify({app:{macOSPrivateApi:false},bundle:{resources:[]}})}});child.on('error',reject);child.on('exit',code=>code===0?resolveBuild():reject(new Error(`Native build failed: ${code}`)));});
}
const reportPath=join(output,'report.json');await unlink(reportPath).catch(()=>{});
const child=spawn(join(root,'src-tauri/target/debug/examples/workbench_native_qa'),[],{cwd:root,env:{...process.env,CODEPET_QA_REPORT:reportPath,CODEPET_QA_EXPECTED:JSON.stringify({mode:'tasks',workspace:root})},stdio:['ignore','pipe','pipe']});
let logs='';child.stdout.on('data',d=>logs+=d);child.stderr.on('data',d=>logs+=d);
const exited=new Promise((resolveExit,reject)=>{child.on('error',reject);child.on('exit',code=>resolveExit(code));});
try{
  let report;const end=Date.now()+95000;
  while(Date.now()<end){try{report=JSON.parse(await readFile(reportPath,'utf8'));break;}catch{}if(child.exitCode!==null)throw new Error(`Native QA exited ${child.exitCode}: ${logs}`);await new Promise(r=>setTimeout(r,100));}
  assert.ok(report,logs);assert.equal(await exited,0,logs);assert.equal(report.success,true,report.error);
  if(report.screenshotTiff){execFileSync('/usr/bin/sips',['-s','format','png',report.screenshotTiff,'--out',join(output,'native-window.png')],{stdio:'pipe'});report.screenshot='native-window.png';}
  await writeFile(reportPath,JSON.stringify(report,null,2));const {taskSnapshot,...summary}=report;console.log(JSON.stringify(summary,null,2));
}finally{if(child.exitCode===null)child.kill();await writeFile(join(output,'native.log'),logs);}
