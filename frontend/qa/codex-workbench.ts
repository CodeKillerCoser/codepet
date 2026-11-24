// Browser-only sample scenarios. Production reads local inventories through Rust IPC.
import '../styles.css';
import '../main-window.css';
import { themeClassNames } from '../lib/theme/defaults';
import { mockIPC } from '@tauri-apps/api/mocks';
import { mount } from 'svelte';
import WorkbenchFrameQa from './WorkbenchFrameQa.svelte';
import type { Snapshot, Worktree } from '../lib/codexWorkbench';
const projects = [{id:'codepet',name:'Code Pet',roots:['/fixture/codepet']},{id:'runtime',name:'Agent Runtime',roots:['/fixture/runtime']},{id:'other',name:'Other project',roots:['/fixture/other']}];
const sourceInfo = {codexHome:'/fixture/.codex',projectCatalog:'database' as const};
const titles = ['完善历史检查工作台','排查 Provider 重连失败','验证工作树改名与暂存区预览','修复主窗口焦点恢复','梳理本地会话与项目的关联'];
const worktrees:Worktree[] = Array.from({length:8},(_,i) => ({
  id:`w${i}`,projectId:i===3?'runtime':'codepet',projectName:i===3?'Agent Runtime':'Code Pet',path:i===0?'/fixture/codepet':`/fixture/worktrees/${i}/codepet`,main:i===0,
  conversations:Array.from({length:i===0?66:i===1?3:i===2||i===5?0:i===7?2:1},(_,j)=>({id:`c${i}-${j}`,source:'Codex',title:titles[j%titles.length],cwd:'/fixture/codepet',updatedAt:new Date('2026-11-24T10:00:00+08:00').getTime()-j*3600000,archived:i===7||(i===0&&j>=60)})),
  git:{branch:['codex/history-workbench','codex/provider-retry','codex/unlinked-work','main','(detached)','','codex/resolve-conflict','codex/archived-history'][i],head:'b38d0aa1',upstream:i===0||i===3,ahead:i===0?2:0,behind:i===3?2:0,
    changes:i===0?[{id:'a',path:'src/history.ts',status:'M.',staged:true},{id:'b',path:'README.md',status:'??',staged:false}]:i===1||i===2?[{id:'a',path:'src/provider.ts',status:'.M',staged:false}]:i===6?[{id:'u',path:'src/conflict.ts',status:'UU',staged:true,conflict:true}]:[],error:i===5?'Git 登记的工作树目录已不存在':undefined}
}));
let data:Snapshot={scanId:1,projects:projects.slice(0,2),worktrees,warnings:[],sourceInfo,scannedAt:new Date('2026-11-24T10:05:00+08:00').getTime()};
let saved=['codepet','runtime'];
(window as any).__workbenchCalls=[];
mockIPC(async(command,args:any)=>{
  if(command.startsWith('plugin:event|')) return 1;
  if(command!=='codex_workbench_request') throw new Error(`Unexpected ${command}`);
  const r=args.request;(window as any).__workbenchCalls.push(r);
  if(r.op==='projects')return {projects,warnings:[],sourceInfo};if(r.op==='preferences')return saved;
  if(r.op==='saveSelection'){saved=r.projectIds;return null;}
  if(r.op==='scan'){await new Promise(resolve=>setTimeout(resolve,(window as any).__scanDelay ?? 80));data={...data,scanId:data.scanId+1,projects:projects.filter(p=>r.projectIds.includes(p.id)),worktrees:worktrees.filter(w=>r.projectIds.includes(w.projectId))};return structuredClone(data);}
  if(r.op==='recheck'){const row=structuredClone(data.worktrees.find(w=>w.id===r.worktreeId)!);row.git.changes=[];row.git.ahead=0;data.worktrees=data.worktrees.map(w=>w.id===row.id?row:w);return row;}
  if(r.op==='preview'){await new Promise(resolve=>setTimeout(resolve,r.version==='workspace'?120:10));return {before:'const message = "before";\n',after:`const message = "${r.version}";\n`};}
  return null;
});
document.getElementById('app')!.className = `main-theme ${themeClassNames('light')}`;
(window as any).__setWorkbenchTheme = (mode: 'light' | 'dark') => document.getElementById('app')!.className = `main-theme ${themeClassNames(mode)}`;
const frame = mount(WorkbenchFrameQa,{target:document.getElementById('app')!});
(window as any).__setWorkbenchMounted = frame.setVisible;
