<script lang="ts">
  import TaskChangeCounts from './TaskChangeCounts.svelte';
  import {changeKinds,type RunChanges,type CommitChange} from './taskChanges';
  import {formatTaskTime, type TaskRun} from './taskLedger';
  export let run:TaskRun;
  export let changes:RunChanges;
  export let commits:CommitChange[]=[];
  export let loading=false;
  export let error='';
  const basename=(path:string)=>path.split('/').at(-1)??path;
  const dirname=(path:string)=>path.slice(0,path.lastIndexOf('/')+1);
</script>
<section class="run-changes" aria-label="执行代码变更">
  <div class="change-summary"><strong>{changes.scope==='run'?'本次执行':changes.scope==='commits'?'关联提交文件':'工作区未提交文件'} · {changes.complete?changes.files.length:changes.files.length?`${changes.files.length} 个已知文件`:'待确认'}</strong><TaskChangeCounts {changes} {loading}/></div>
  <p class="context">{run.title}</p>
  <p class="context path">{run.agentWorkspace.path}</p>
  {#if error}<p role="alert" class="read-error">{error}</p>{/if}
  {#if run.agentWorkspace.error}<p class="context">{run.agentWorkspace.error}</p>{/if}
  <div class="change-file-list">
    {#each changes.files as file (file.path)}
      <div class="change-file" title={file.originalPath?`${file.originalPath} → ${file.path}`:file.path}>
        <span class="file-path"><strong>{basename(file.path)}</strong><small>{dirname(file.path)}{file.originalPath?` ← ${file.originalPath}`:''}</small></span>
        <span class={`file-status ${file.changeType}`} aria-label={changeKinds[file.changeType].label} title={changeKinds[file.changeType].label}>{changeKinds[file.changeType].symbol}</span>
      </div>
    {:else}{#if !loading}<p class="context">{changes.scope==='run'?'本次执行没有代码变更':error?'未能读取文件列表':'没有记录到变更文件'}</p>{/if}{/each}
  </div>
  {#if run.agentWorkspace.relatedCommits.length}
    <h4>关联提交 · {run.agentWorkspace.relatedCommits.length}</h4>
    {#each run.agentWorkspace.relatedCommits as sha}
      {@const commit=commits.find(c=>c.commit===sha)}
      <details class="commit-changes"><summary><code title={sha}>{sha.slice(0,8)}</code><small>{commit?`${commit.files.length} 个文件`:loading?'读取中…':'文件未读取'}</small></summary>
        {#if commit?.error}<p class="read-error">{commit.error}</p>{:else if commit}
          {#each commit.files as file}<div class="commit-file"><code>{file.originalPath?`${file.originalPath} → `:''}{file.path}</code><small>{file.changeType==='untracked'?'A':changeKinds[file.changeType as keyof typeof changeKinds]?.symbol??'?'}</small></div>{/each}
        {/if}
      </details>
    {/each}
  {/if}
  <p class="context observation">工作区观测于 {formatTaskTime(run.agentWorkspace.observedAt)}。{changes.scope==='workspace'?'未提交文件属于共享工作区。':''}{changes.counts.unknown?' 未确认项仅有路径申报，尚无可核对的 Git 类型。':''}</p>
</section>
<style>
 .run-changes{color:var(--app-text);font:12px/1.6 var(--font-family-ui);min-width:0}
 .change-summary{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}.change-summary strong{font-weight:500}
 .context{color:var(--app-muted);font-size:11px;margin:6px 0;overflow-wrap:anywhere}.path,code{font-family:var(--font-family-code);font-size:11px;overflow-wrap:anywhere}
 .change-file-list{margin-top:16px}.change-file{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:12px;align-items:center;padding:7px 0}
 .file-path{min-width:0}.file-path strong{font-size:12px;font-weight:500;overflow-wrap:anywhere}.file-path small{display:block;color:var(--app-muted);font:10px/1.5 var(--font-family-code);overflow-wrap:anywhere}
 .file-status{font:500 11px/1.5 var(--font-family-code);color:var(--app-muted)}.added{color:var(--color-success)}.modified,.renamed{color:var(--wb-info-text)}.deleted,.conflicted,.read-error{color:var(--app-danger)}
 .read-error{font-size:11px;overflow-wrap:anywhere}h4{font-size:12px;font-weight:500;margin:22px 0 8px}.commit-changes{padding:6px 0}.commit-changes summary{display:flex;gap:12px;align-items:center;cursor:pointer;list-style:none}.commit-changes summary::-webkit-details-marker{display:none}.commit-changes small{font-size:10px;color:var(--app-muted)}
 .commit-file{display:flex;gap:8px;justify-content:space-between;padding:4px 0}.commit-file code{min-width:0}.observation{margin-top:20px}
</style>
