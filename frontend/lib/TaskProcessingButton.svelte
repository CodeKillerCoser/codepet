<script lang="ts">
  import {LoaderCircle, Sparkles} from '@lucide/svelte';
  import {historyRequest} from './recallHistory';
  import type {Task} from './taskLedger';
  export let task:Task;
  export let disabled=false;
  export let onFeedback:(message:string,error:boolean)=>void=()=>{};
  let submitting=false;
  let pending:{taskId:string;operationId:string}|undefined;
  $: eligible=task.runs.some(run=>run.status!=='in_progress');
  async function process(){
    if(submitting||disabled||!eligible)return;
    if(pending?.taskId!==task.id)pending={taskId:task.id,operationId:crypto.randomUUID()};
    submitting=true;onFeedback('',false);
    try{
      const receipt=await historyRequest<{queued:number;existing:number}>('enqueue',pending);
      pending=undefined;
      onFeedback([receipt.queued?`${receipt.queued} 次执行已加入加工队列`:'',receipt.existing?`${receipt.existing} 次执行已在排队或加工中`:''].filter(Boolean).join('；')+'。可在任务图谱的加工历史查看进度。',false);
    }catch(cause){onFeedback(cause instanceof Error?cause.message:String(cause),true);}
    finally{submitting=false;}
  }
</script>

<button type="button" class="quiet process-task" disabled={disabled||submitting||!eligible} aria-label={`加工任务：${task.title}`} aria-busy={submitting}
  title={!eligible?'完成开发后可加工':submitting?'正在加入加工队列…':'加工任务的所有已结束执行；已完成索引将重新加工'} on:click={process}>
  {#if submitting}<LoaderCircle size={16} class="processing"/>{:else}<Sparkles size={16}/>{/if}
</button>

<style>
  .process-task{display:inline-flex;align-items:center;justify-content:center;flex:none;width:30px;height:30px;min-height:30px;padding:0;border:0;border-radius:6px;background:transparent;color:var(--app-muted);cursor:pointer;}
  .process-task:disabled{opacity:.45;cursor:default;}.process-task:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px;}
  :global(.process-task .processing){animation:processing-turn 1s linear infinite;}@keyframes processing-turn{to{transform:rotate(360deg);}}
  @media(prefers-reduced-motion:reduce){:global(.process-task .processing){animation:none;}}
</style>
