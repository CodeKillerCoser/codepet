import {actorLabel, fileLabels, kinds, results, scopes, statuses, type TaskActivity, type TaskRun} from './taskLedger';
export const activityLabels:Record<string,string>={created:'创建执行',comment:'评论',content_updated:'修改执行',workspace_changed:'切换工作区',workspace_observed:'代码观测',plan_updated:'修改计划',verification:'验证结果',delivery:'发布记录',status_changed:'状态变更',finished:'开发结论',closed:'结束执行',task_completed:'完成任务',task_edited:'修改任务',conversation_linked:'关联对话',milestone:'进展',workspace_snapshot:'历史观测',imported_delivery:'历史发布'};
export const activityFields:Record<string,string>={title:'执行标题',kind:'执行类型',goal:'目标',solution:'方案',description:'说明',status:'状态',validationPlan:'验收计划',verification:'验证记录',agentWorkspace:'工作区与代码证据',agentConversation:'关联对话',closure:'结束记录',taskStatus:'任务状态',taskTitle:'任务标题',taskDescription:'任务背景'};
const object=(value:unknown):Record<string,any>=>value&&typeof value==='object'&&!Array.isArray(value)?value as Record<string,any>:{};
export function activityValue(field:string,value:unknown,run?:TaskRun):string{
  if(value===null||value===undefined)return '无';
  if(field==='status'&&typeof value==='string')return statuses[value as keyof typeof statuses]??value;
  if(field==='kind'&&typeof value==='string')return kinds[value as keyof typeof kinds]??value;
  if(typeof value==='string')return value||'空';
  const data=object(value);
  if(field==='validationPlan')return [`第 ${data.revision} 版`,...(data.items??[]).map((item:any)=>[item.title,item.precondition?`前提：${item.precondition}`:'',`验收方式：${item.steps}`,`预期：${item.expected}`,`${scopes[item.scope as keyof typeof scopes]??item.scope} · ${item.required?'必验':'可选'}`].filter(Boolean).join('\n'))].join('\n');
  if(field==='verification'){
    const input=data.input??data;
    const criteria=run?.validationPlan.revisions.find(plan=>plan.revision===input.planRevision)?.criteria??run?.validationPlan.items??[];
    return [`${scopes[input.stage as keyof typeof scopes]??input.stage} · ${input.target} · ${input.environment}`,...(input.results??[]).map((check:any)=>`${criteria.find(item=>item.id===check.criterionId)?.title??check.criterionId} · ${results[check.result as keyof typeof results]??check.result}：${check.evidence}`)].join('\n');
  }
  if(field==='agentConversation'&&Array.isArray(value))return value.map(item=>`${actorLabel(item.agent)} · ${item.conversationId}`).join('\n')||'无';
  if(field==='taskStatus')return `${statuses[data.status as keyof typeof statuses]??String(data.status)}${data.manuallyCompleted?'（人工完成）':''}`;
  if(field==='closure')return `${data.accepted?'人工验收通过':'注明原因结束'}：${data.reason??''}`;
  if(field==='agentWorkspace')return [data.path,data.branch?`分支：${data.branch}`:'',data.head?`提交：${data.head}`:'',
    ...(data.relatedCommits??[]).map((sha:string)=>`关联提交：${sha}`),...(data.runFiles??[]).map((path:string)=>`执行文件：${path}`),
    `未提交改动：${data.changeStatus?.hasUncommittedChanges===null?'未知':data.changeStatus?.hasUncommittedChanges?'有':'无'}`,
    `待推送提交：${data.changeStatus?.hasUnpushedCommits===null?'未知':data.changeStatus?.hasUnpushedCommits?'有':'无'}`,
    ...(data.changedFiles??[]).map((file:any)=>`${fileLabels[file.changeType]??'工作区文件'}：${file.path}`),data.error??''].filter(Boolean).join('\n');
  return JSON.stringify(value,null,2);
}
export function replyLabel(entry:TaskActivity|undefined){return entry?`${actorLabel(entry.actor)}：${entry.summary}`:'历史活动';}
