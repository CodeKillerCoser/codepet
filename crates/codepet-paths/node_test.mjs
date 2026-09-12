import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,rmSync} from 'node:fs';
import {join,dirname,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {getPaths} from './node.mjs';
test('Node path manager reads shared JSON and keeps workspace independent', t => {
 const root=mkdtempSync(join(tmpdir(),'codepet-path-manager-'));
 t.after(()=>{if(dirname(resolve(root))!==resolve(tmpdir()))throw new Error('unsafe test path');rmSync(root,{recursive:true,force:true});});
 const env={LOCALAPPDATA:root};const initial=getPaths({home:root,platform:'win32',env});
 assert.equal(initial.data,join(root,'code-pet'));assert.equal(initial.workspace,join(root,'.codepet'));
 mkdirSync(dirname(initial.settingsFile),{recursive:true});writeFileSync(initial.settingsFile,JSON.stringify({data:{dataDirectory:join(root,'custom')},appearance:{theme:'dark'}}));
 const changed=getPaths({home:root,platform:'win32',env});assert.equal(changed.data,join(root,'custom'));assert.equal(changed.workspace,initial.workspace);assert.equal(changed.spool,join(root,'custom','spool','events.jsonl'));
 writeFileSync(initial.settingsFile,'broken');assert.throws(()=>getPaths({home:root,platform:'win32',env}));
 const mac=getPaths({home:root,platform:'darwin',env:{}});assert.equal(mac.data,join(root,'Library','Application Support','code-pet'));
});
