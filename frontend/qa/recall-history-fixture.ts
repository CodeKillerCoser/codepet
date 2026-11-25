// Development-only IPC fixtures: queue/error scenarios never modify the real catalog.
import type { ProcessingHistory, ProcessingJob } from '../lib/recallHistory';
let jobs: ProcessingJob[] | undefined;
async function initialize() {
  if (jobs) return jobs;
  const now = Date.now();
  const base: ProcessingJob = {id:'00000000-0000-4000-8000-000000000001',taskId:'00000000-0000-4000-8000-000000000002',runId:'00000000-0000-4000-8000-000000000003',title:'接入任务语义召回',taskTitle:'CodePet 任务召回',workspace:'/fixture/codepet',contentHash:'fixture-content-version',fingerprint:'fixture-profile',state:'processing',queuedAt:now-60000,startedAt:now-32000,finishedAt:null,elapsedMs:32000,phase:'加工文档与实体',error:null,resultSummary:'',models:{llm:'Qwen3.8 27B',embedding:'Qwen3 Embedding 0.6B'},usageTracked:true,usage:{models:[],requests:0,reportedRequests:0,inputTokens:0,outputTokens:0,knownTokens:0,complete:true},canCancel:false,canRetry:false};
  let completed: ProcessingJob | undefined;
  try {
    const report=await fetch('/artifacts/task-recall-sidecar-qa/mcp-recall-report.json').then(r=>r.json());
    if(report.processing) completed={...report.processing,usage:{models:[],...report.processingUsage,knownTokens:report.processingUsage.inputTokens+report.processingUsage.outputTokens,complete:report.processingUsage.requests===report.processingUsage.reportedRequests}};
  } catch {}
  jobs=[base,{...base,id:'00000000-0000-4000-8000-000000000004',runId:'00000000-0000-4000-8000-000000000005',title:'模型配置页面重设计',state:'queued',startedAt:null,elapsedMs:null,canCancel:true},
    completed??{...base,id:'00000000-0000-4000-8000-000000000006',runId:'00000000-0000-4000-8000-000000000007',title:'修复任务状态汇总',state:'completed',finishedAt:now-240000,elapsedMs:48000,phase:'加工完成',usageTracked:false,resultSummary:'旧索引已完成，未记录加工详情'},
    {...base,id:'00000000-0000-4000-8000-000000000008',runId:'00000000-0000-4000-8000-000000000009',title:'调整知识图谱预览',state:'failed',finishedAt:now-900000,elapsedMs:32000,error:'TimeoutError',canRetry:true,usageTracked:false}];
  if(new URLSearchParams(location.search).has('historyLong')) jobs[0]={...jobs[0],title:'验证超长任务名称在全局侧栏中保持单行省略并且不会遮挡弹出详情的操作入口'.repeat(3),taskTitle:'跨工作区任务主题'.repeat(20),phase:'模型正在读取超长路径与提取实体关系'.repeat(20)};
  return jobs;
}
export async function recallHistoryFixture(request: Record<string,any>) {
  const options = new URLSearchParams(location.search);
  if(options.has('historyError')) throw new Error('加工服务暂时无法连接（界面验证）');
  const rows=await initialize();
  if(request.action==='detail') {
    const job=rows.find(row=>row.id===request.jobId);if(!job) throw new Error('记录不存在');return job;
  }
  if(request.action==='cancel'||request.action==='retry') {
    if(options.has('historyConflict')) throw new Error('加工队列已变化，请刷新后重试');
    const index=rows.findIndex(row=>row.id===request.jobId);if(index<0)throw new Error('记录不存在');
    rows[index]={...rows[index],state:request.action==='cancel'?'cancelled':'queued',canCancel:request.action==='retry',canRetry:request.action==='cancel',finishedAt:request.action==='cancel'?Date.now():null};
    return {id:request.jobId,state:rows[index].state};
  }
  const all=options.has('historyEmpty')?[]:rows;
  const filtered=all.filter(row=>!request.state||row.state===request.state);
  const offset=request.offset??0,limit=request.limit??40;
  return {items:filtered.slice(offset,offset+limit),total:filtered.length,offset,hasMore:offset+limit<filtered.length,
    counts:all.reduce((counts,row)=>({...counts,[row.state]:(counts[row.state]??0)+1}),{} as Record<string,number>)} satisfies ProcessingHistory;
}
