// This evaluation's build target is macOS arm64; production platform packaging is unverified.
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createHash } from 'node:crypto';
import { gunzipSync } from 'node:zlib';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

if(process.platform!=='darwin'||process.arch!=='arm64') throw new Error('This probe currently verifies macOS arm64 only');
const exec=promisify(execFile),source=dirname(fileURLToPath(import.meta.url));
const root=resolve(process.argv[2]||'artifacts/cognee-task-recall-qa/native-comparison'),vendor=join(root,'vendor');
await mkdir(vendor,{recursive:true});
const downloads=[
  ['ladybug.tar.gz','https://github.com/LadybugDB/ladybug/releases/download/v0.22.3/liblbug-osx-arm64.tar.gz','b60a13f4a5aeba0f10e68be8c524a2fd3920a8a683208fcee8507461537a92a3'],
  ['cozo.dylib.gz','https://github.com/cozodb/cozo/releases/download/v0.7.6/libcozo_c-0.7.6-aarch64-apple-darwin.dylib.gz','3f654101b5a7d942325ec7d5809e0ddd9175bbfd55669b566c152f48e663e4e8'],
  ['sqlite-vec.tar.gz','https://github.com/asg017/sqlite-vec/releases/download/v0.1.9/sqlite-vec-0.1.9-amalgamation.tar.gz','3acd67cb4aff080c7050926fd3cf8227905fe5b7ee3829d8ee5024ab1283cf61'],
  ['json.hpp','https://raw.githubusercontent.com/nlohmann/json/v3.12.0/single_include/nlohmann/json.hpp','aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63'],
  ['cozo_c.h','https://raw.githubusercontent.com/cozodb/cozo/v0.7.6/cozo-lib-c/cozo_c.h',null],
];
for(const [name,url,expected] of downloads) {
  const path=join(vendor,name);
  try { await readFile(path); } catch { await exec('curl',['-fsSL','--retry','2','--connect-timeout','15','--max-time','90',url,'-o',path]); }
  const data=await readFile(path),sha256=createHash('sha256').update(data).digest('hex');
  if(expected&&sha256!==expected) throw new Error(`checksum mismatch: ${name}`);
  console.log(JSON.stringify({name,bytes:data.length,sha256,url}));
}
for(const name of ['ladybug.tar.gz','sqlite-vec.tar.gz']) await exec('tar',['-xzf',join(vendor,name),'-C',vendor]);
await writeFile(join(vendor,'libcozo_c.dylib'),gunzipSync(await readFile(join(vendor,'cozo.dylib.gz'))));
await exec('clang',['-O2','-DSQLITE_CORE','-DSQLITE_VEC_STATIC','-c',join(vendor,'sqlite-vec.c'),'-o',join(vendor,'sqlite-vec.o')]);
const common=['-O2','-std=c++17',`-I${vendor}`,join(source,'driver.cpp')];
await Promise.all([
  exec('clang++',[...common,'-DENGINE_SQLITE','-DSQLITE_CORE','-DSQLITE_VEC_STATIC',join(vendor,'sqlite-vec.o'),'-lsqlite3','-o',join(root,'sqlite-driver')]),
  exec('clang++',[...common,'-DENGINE_COZO',`-L${vendor}`,'-lcozo_c','-Wl,-rpath,@executable_path/vendor','-o',join(root,'cozo-driver')]),
  exec('clang++',[...common,`-L${vendor}`,'-llbug','-Wl,-rpath,@executable_path/vendor','-o',join(root,'ladybug-driver')]),
]);
await exec('install_name_tool',['-change','/Volumes/Corpora/repos/cozo/target/aarch64-apple-darwin/release/deps/libcozo_c.dylib','@rpath/libcozo_c.dylib',join(root,'cozo-driver')]);
await exec('codesign',['--force','--sign','-',join(root,'cozo-driver')]);
await exec('npm',['install','--prefix',join(root,'helix'),'--no-audit','--no-fund','@helix-db/helix-db@3.2.0','@helix-db/helix-db-embedded@0.3.3']);
console.log('Native drivers built. Install Ladybug vector and fts once using INSTALL vector / INSTALL fts, then LOAD them per connection.');
