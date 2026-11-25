<script lang="ts">
 import {onDestroy} from 'svelte';
 import {taskRequest} from './taskLedger';
 import {patchLines,type FileDiff,type DiffLine} from './taskDiffs';
 export let taskId:string;export let runId:string;export let version:string;export let path:string;
 let loading=true,error='',lines:DiffLine[]=[],patch='',sequence=0;
 $: void load(taskId,runId,version,path);
 async function load(taskId:string,runId:string,version:string,path:string){
  const seq=++sequence;loading=true;error='';lines=[];patch='';
  try{const result=await taskRequest<FileDiff>('file-diff',{taskId,runId,version,path});if(seq!==sequence)return;error=result.error??'';patch=result.patch;lines=patchLines(patch);}
  catch(e){if(seq===sequence)error=String(e);}finally{if(seq===sequence)loading=false;}
 }
 onDestroy(()=>++sequence);
</script>
<div class="file-diff" aria-label={`${path} 的代码差异`} aria-busy={loading}>
 {#if loading}<p role="status">读取差异…</p>
 {:else if error}<p class="diff-error" role="status">{error}</p>
 {:else if !lines.length}<p>{patch?'仅文件名或属性变更，没有文本行差异':'没有文本行差异'}</p>
 {:else}
 <div class="diff-legend" aria-hidden="true"><span>原行</span><span>新行</span><span>− 删除 / + 新增</span></div>
 <div class="diff-lines" role="textbox" aria-readonly="true" aria-multiline="true" aria-label="只读代码差异" tabindex="0">
  {#each lines as line}<div class={`diff-line ${line.kind}`}><span class="line-number">{line.old??''}</span><span class="line-number">{line.next??''}</span><span class="line-sign" aria-hidden="true">{line.kind==='added'?'+':line.kind==='deleted'?'−':''}</span><code>{line.text||' '}</code></div>{/each}
 </div>
 {/if}
</div>
<style>
 .file-diff{margin:0 0 12px;min-width:0;border-radius:4px;overflow:hidden;background:var(--wb-bg);color:var(--wb-text);}
 p{padding:10px;margin:0;font:var(--font-size-caption)/var(--line-height-body) var(--font-family-ui);overflow-wrap:anywhere}.diff-error{color:var(--wb-danger-text)}
 .diff-legend{display:grid;grid-template-columns:34px 34px minmax(0,1fr);gap:0;padding:4px 0;color:var(--task-secondary);font:var(--font-size-xs)/var(--line-height-body-relaxed) var(--font-family-ui)}.diff-legend span{text-align:right}.diff-legend span:last-child{text-align:left;padding-left:12px}
 .diff-lines{max-height:none;overflow:auto;min-width:0;outline-offset:-2px}.diff-line{display:grid;grid-template-columns:34px 34px 16px minmax(0,1fr);font:var(--font-size-code)/var(--line-height-body-relaxed) var(--font-family-code);}
 .line-number{user-select:none;text-align:right;padding-right:5px;color:var(--task-secondary);font-variant-numeric:tabular-nums}.line-sign{user-select:none;text-align:center;color:var(--task-secondary)}
 code{white-space:pre-wrap;overflow-wrap:anywhere;tab-size:2;padding-right:6px;min-width:0;font:inherit}
 .added{background:var(--wb-success-bg)}.deleted{background:var(--wb-danger-bg)}.added .line-sign{color:var(--color-success)}.deleted .line-sign{color:var(--wb-danger-text)}.hunk,.note{color:var(--task-secondary);background:var(--wb-control)}.hunk code,.note code{font-size:var(--font-size-code)}
</style>
