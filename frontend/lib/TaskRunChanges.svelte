<script lang="ts">
  import {onDestroy} from 'svelte';
  import {FileCode,ChevronRight} from '@lucide/svelte';
  import TaskChangeCounts from './TaskChangeCounts.svelte';
  import TaskWorkspaceStatus from './TaskWorkspaceStatus.svelte';
  import TaskFileDiff from './TaskFileDiff.svelte';
  import {changeKinds,type RunChanges,type CommitChange} from './taskChanges';
  import {formatTaskTime,taskRequest,type TaskRun} from './taskLedger';
  import type {DiffVersion,DiffFile} from './taskDiffs';
  export let taskId:string;
  export let run:TaskRun;
  export let changes:RunChanges;
  export let commits:CommitChange[]=[];
  export let loading=false;
  export let error='';
  let files:DiffFile[]=[];
  let versions:DiffVersion[]=[],selected='',pending=true,diffError='',sequence=0,expanded=new Set<string>();
  const basename=(path:string)=>path.split('/').at(-1)??path;
  const dirname=(path:string)=>path.slice(0,path.lastIndexOf('/')+1);
  $: void load(taskId,run.id,run.agentWorkspace.observedAt,run.agentWorkspace.relatedCommits.join('|'));
  $: version=versions.find(v=>v.id===selected)??versions.at(-1);
  $: files=(version?.files??changes.files.map(f=>({...f,added:null,deleted:null,error:null}))).filter(f=>run.agentWorkspace.runFiles==null||run.agentWorkspace.runFiles.includes(f.path));
  async function load(taskId:string,runId:string,_observedAt:number,_commits:string){
    const seq=++sequence;pending=true;diffError='';versions=[];selected='';expanded=new Set();
    try{const value=await taskRequest<DiffVersion[]>('change-diffs',{taskId,runId});if(seq===sequence){versions=value;selected=value.at(-1)?.id??'';}}
    catch(e){if(seq===sequence)diffError=String(e);}finally{if(seq===sequence)pending=false;}
  }
  function toggle(path:string,open:boolean){const next=new Set(expanded);if(open)next.add(path);else next.delete(path);expanded=next;}
  onDestroy(()=>++sequence);
</script>
<section class="run-changes" aria-label="执行代码变更">
  <div class="change-summary"><strong>{changes.scope==='run'?'本次执行':changes.scope==='commits'?'关联提交文件':'工作区未提交文件'} · {changes.complete?changes.files.length:changes.files.length?`${changes.files.length} 个已知文件`:'待确认'}</strong><TaskChangeCounts {changes} {loading}/></div>
  <div class="context change-context"><code>{run.agentWorkspace.branch??'分支未知'}</code><TaskWorkspaceStatus workspace={run.agentWorkspace}/></div>
  <p class="context path" title={run.agentWorkspace.path}>{run.agentWorkspace.path}</p>
  {#if error}<p role="alert" class="read-error">{error}</p>{/if}
  {#if diffError}<p role="alert" class="read-error">差异读取失败：{diffError}</p>{/if}
  {#if versions.length}
    <div class="version-choice"><select aria-label="变更版本" title={`状态记录于 ${formatTaskTime(run.agentWorkspace.observedAt)}；已保存的差异在提交、推送和清理后仍保留。`} bind:value={selected} on:change={()=>expanded=new Set()}>
      {#each versions as entry}<option value={entry.id}>{entry.commit?`提交 ${entry.commit.slice(0,8)}`:`工作区快照 · ${formatTaskTime(entry.observedAt)}`}</option>{/each}
    </select></div>
  {:else if pending}<p class="context" role="status">读取差异快照…</p>
  {:else if !diffError&&files.length}<p class="context">这些旧记录未保存 diff；关联提交可读取时会自动补存。</p>{/if}
  {#if version?.error}<p class="read-error">{version.error}</p>{/if}
  {#key version?.id??'unavailable'}
  <div class="change-file-list">
    {#each files as file (file.path)}
      <details class="change-file" title={file.originalPath?`${file.originalPath} → ${file.path}`:file.path} on:toggle={event=>toggle(file.path,event.currentTarget.open)}>
        <summary aria-label={`查看 ${file.path} 的差异`}>
          <ChevronRight class="file-chevron" size={12}/><FileCode size={14}/>
          <span class="file-path"><strong>{basename(file.path)}</strong><small>{dirname(file.path)}{file.originalPath?` ← ${file.originalPath}`:''}</small></span>
          <span class="line-counts">{#if file.added!==null}<span class="added">+{file.added}</span><span class="deleted">−{file.deleted}</span>{/if}</span>
          <span class={`file-status ${file.changeType}`} aria-label={changeKinds[file.changeType as keyof typeof changeKinds]?.label??'未知'}>{changeKinds[file.changeType as keyof typeof changeKinds]?.symbol??'?'}</span>
        </summary>
        {#if expanded.has(file.path)}
          {#if version}<TaskFileDiff {taskId} runId={run.id} version={version.id} path={file.path}/>{:else}<p class="context">尚无可用的差异快照。</p>{/if}
        {/if}
      </details>
    {:else}{#if !loading&&!pending}<p class="context">{version&&changes.files.length?'该版本不含本次执行的文件变更':changes.scope==='run'?'本次执行没有代码变更':error?'未能读取文件列表':'没有记录到变更文件'}</p>{/if}{/each}
  </div>
  {/key}
  {#if run.agentWorkspace.relatedCommits.length}
    <h4>关联提交 · {run.agentWorkspace.relatedCommits.length}</h4>
    {#each run.agentWorkspace.relatedCommits as sha}
      {@const commit=commits.find(c=>c.commit===sha)}
      <p class="commit-reference"><code title={sha}>{sha.slice(0,8)}</code><small>{commit?`${commit.files.length} 个文件`:loading?'读取中…':'文件未读取'}</small></p>
    {/each}
  {/if}
</section>
<style>
 .run-changes{--task-secondary:color-mix(in srgb,var(--app-text) 76%,var(--wb-panel));color:var(--app-text);font:var(--font-size-label)/var(--line-height-body) var(--font-family-ui);min-width:0}
 .change-summary{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}.change-summary strong{font-weight:var(--font-weight-medium)}
 .change-context{display:flex;align-items:center;gap:8px 14px;flex-wrap:wrap;}
 .context{color:var(--task-secondary);font-size:var(--font-size-sm);margin:6px 0;overflow-wrap:anywhere}.path,code{font-family:var(--font-family-code);font-size:var(--font-size-sm);overflow-wrap:anywhere}
 .version-choice{display:flex;align-items:center;gap:10px;font-size:var(--font-size-sm);color:var(--task-secondary);margin-top:18px}.version-choice select{min-width:0;flex:1;background:var(--wb-panel);color:var(--wb-text);font:inherit;border:1px solid var(--wb-border);border-radius:4px;padding:5px}
 .change-file-list{margin-top:14px}.change-file{min-width:0}.change-file summary{display:grid;grid-template-columns:12px 14px minmax(0,1fr) auto 14px;gap:6px;align-items:center;padding:8px 2px;cursor:pointer;list-style:none;border-radius:4px}.change-file summary:hover{background:var(--wb-control)}.change-file summary::-webkit-details-marker{display:none}.change-file[open] :global(.file-chevron){transform:rotate(90deg)}
 .file-path{min-width:0}.file-path strong{display:block;font-size:var(--font-size-caption);font-weight:var(--font-weight-medium);overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.file-path small{display:block;color:var(--task-secondary);font:var(--font-size-xs)/var(--line-height-loose) var(--font-family-code);overflow-wrap:anywhere}
 .file-status,.line-counts{font:var(--font-weight-medium) var(--font-size-sm)/var(--line-height-loose) var(--font-family-code);font-variant-numeric:tabular-nums}.line-counts{display:flex;gap:6px}.file-status{color:var(--task-secondary)}.added{color:var(--color-success)}.modified,.renamed{color:var(--wb-info-text)}.deleted,.conflicted,.read-error{color:var(--app-danger)}
 .read-error{font-size:var(--font-size-caption);overflow-wrap:anywhere}h4{font:var(--font-subheading);margin:22px 0 8px}.commit-reference{display:flex;gap:12px;margin:6px 0}.commit-reference small{font-size:var(--font-size-caption);color:var(--task-secondary)}
</style>
