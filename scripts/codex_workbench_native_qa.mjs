// Read-only native acceptance of this workspace. Does not use browser mocks or stop the installed app.
import { build } from 'vite';
import { execFileSync, spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, unlink, realpath } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import assert from 'node:assert/strict';
const root=resolve(import.meta.dirname,'..');
const output=join(root,'artifacts/codex-workbench-qa/native');await mkdir(output,{recursive:true});
const workspace=await realpath(process.env.CODEPET_QA_WORKSPACE||root);
const threadId=process.env.CODEX_THREAD_ID;
assert.ok(threadId,'Run from the Codex chat being verified (CODEX_THREAD_ID is required)');
const codexHome=process.env.CODEX_HOME||join(homedir(),'.codex');
const projectId=execFileSync('python3',['-c',`
import sqlite3,subprocess,pathlib,sys
home,workspace=sys.argv[1:]
def common(path):
 s=subprocess.check_output(['git','-C',path,'rev-parse','--git-common-dir'],text=True,stderr=subprocess.DEVNULL).strip()
 return (pathlib.Path(path)/s).resolve()
target=common(workspace)
c=sqlite3.connect('file:'+str(pathlib.Path(home)/'state_5.sqlite')+'?mode=ro',uri=True)
ids=set()
for id,path in c.execute('select project_id,path from project_roots'):
 try:
  if common(path)==target:ids.add(id)
 except (OSError,subprocess.CalledProcessError):pass
if len(ids)!=1:raise SystemExit('Expected a unique Codex project for this Git repository, got '+str(len(ids)))
print(next(iter(ids)))
`,codexHome,workspace],{encoding:'utf8'}).trim();
if(!process.argv.includes('--skip-build')){
 await build({root,build:{outDir:join(output,'site'),emptyOutDir:true,rollupOptions:{input:join(root,'frontend/qa/codex-workbench-native.html')}}});
 await new Promise((resolveBuild,reject)=>{const child=spawn('cargo',['build','--manifest-path','src-tauri/Cargo.toml','--example','workbench_native_qa','--features','custom-protocol'],{cwd:root,stdio:'inherit',env:{...process.env,TAURI_CONFIG:JSON.stringify({app:{macOSPrivateApi:false},bundle:{resources:[]}})}});child.on('error',reject);child.on('exit',code=>code===0?resolveBuild():reject(new Error(`Native QA build failed: ${code}`)));});
}
// Independent Git v1 baseline; production scanner parses v2. Keep rename source records paired.
const records=execFileSync('git',['-C',workspace,'status','--porcelain=v1','-z','--untracked-files=normal'],{env:{...process.env,GIT_OPTIONAL_LOCKS:'0',GIT_NO_LAZY_FETCH:'1'}}).toString().split('\0').filter(Boolean);
let gitEntries=0;for(let i=0;i<records.length;i++){gitEntries++;if(/[RC]/.test(records[i].slice(0,2)))i++;}
assert.ok(gitEntries>0,'The current acceptance scenario requires a dirty worktree');
const reportPath=join(output,'report.json');await unlink(reportPath).catch(()=>{});
const child=spawn(join(root,'src-tauri/target/debug/examples/workbench_native_qa'),[],{cwd:root,env:{...process.env,CODEPET_QA_REPORT:reportPath,CODEPET_QA_EXPECTED:JSON.stringify({workspace,threadId,projectId,gitEntries})},stdio:['ignore','pipe','pipe']});
let logs='';child.stdout.on('data',chunk=>{logs+=chunk;});child.stderr.on('data',chunk=>{logs+=chunk;process.stderr.write(chunk);});
const exit=new Promise((resolveExit,reject)=>{child.on('error',reject);child.on('exit',code=>resolveExit(code));});
let report;
try{
 const deadline=Date.now()+95000;
 while(Date.now()<deadline){try{report=JSON.parse(await readFile(reportPath,'utf8'));break;}catch{}if(child.exitCode!==null)throw new Error(`Native QA exited ${child.exitCode} without report: ${logs}`);await new Promise(r=>setTimeout(r,100));}
 assert.ok(report,`Native QA did not produce a report: ${logs}`);
 if(report.screenshotTiff){execFileSync('/usr/bin/sips',['-s','format','png',report.screenshotTiff,'--out',join(output,'native-window.png')],{stdio:'pipe'});report.screenshot='native-window.png';}
 if(report.windowNumber){try{execFileSync('/usr/sbin/screencapture',['-x','-l',String(report.windowNumber),join(output,'native-window.png')],{stdio:'pipe'});report.screenshot='native-window.png';}catch(e){report.screenshotError=String(e.stderr||e.message);}}
 const code=await exit;await writeFile(join(output,'native.log'),logs);await writeFile(reportPath,JSON.stringify(report,null,2));
 assert.equal(code,0,logs);assert.equal(report.success,true,report.error);assert.equal(report.workspace,workspace);assert.equal(report.gitEntryCount,gitEntries);assert.equal(report.currentThreadAssociated,true);assert.equal(report.previewWorked,true);
 console.log(JSON.stringify(report,null,2));
}catch(error){if(child.exitCode===null)child.kill('SIGTERM');await writeFile(join(output,'native.log'),logs);throw error;}
