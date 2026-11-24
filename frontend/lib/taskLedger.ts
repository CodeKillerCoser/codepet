import { invoke } from '@tauri-apps/api/core';
export type RunStatus='in_progress'|'ready_for_delivery'|'published'|'closed';
export type Scope='local'|'release'|'both';
export type TestResult='passed'|'failed'|'not_tested'|'blocked';
export type TaskKind='bugfix'|'feature';
export const statuses:Record<RunStatus,string>={in_progress:'进行中',ready_for_delivery:'待交付',published:'已发布',closed:'已结束'};
export const phases={in_progress:'进行中',ready_for_delivery:'待交付',published:'已发布'};
export const kinds:Record<TaskKind,string>={bugfix:'修复',feature:'功能'};
export const results:Record<TestResult,string>={passed:'通过',failed:'失败',not_tested:'未测',blocked:'受阻'};
export const scopes:Record<Scope,string>={local:'本地',release:'发布后',both:'本地与发布后'};
export interface Criterion{id:string;title:string;precondition:string;steps:string;expected:string;scope:Scope;required:boolean}
export interface AgentWorkspace{path:string;branch:string|null;head:string|null;upstream:string|null;changeStatus:{hasUncommittedChanges:boolean|null;hasUnpushedCommits:boolean|null};changedFiles:{path:string;changeType:string;staged:boolean;originalPath:string|null}[];relatedCommits:string[];observedAt:number;error:string|null}
export interface VerificationInput{planRevision:number;stage:Scope;target:string;environment:string;results:{criterionId:string;result:TestResult;evidence:string}[]}
export interface Verification extends VerificationInput{id:string;actor:string;agent:string;createdAt:number;agentWorkspace:AgentWorkspace|null}
export interface PlanRevision{revision:number;createdAt:number;summary:string;affectedAreas:string[];criteria:Criterion[]}
export interface TaskRun{id:string;title:string;kind:TaskKind;goal:string;solution:string;description:string;status:RunStatus;validationPlan:{revision:number;items:Criterion[];records:Verification[];revisions:PlanRevision[]};agentWorkspace:AgentWorkspace;agentConversation:{agent:string;conversationId:string;verified:boolean}[];timeline:{id:string;at:number;actor:string;kind:string;summary:string;reference?:string;agentWorkspace?:AgentWorkspace}[];createdBy:string;createdAt:number;updatedAt:number;closure:{accepted:boolean;reason:string;closedAt:number}|null}
export interface Task{schemaVersion:number;id:string;title:string;description:string;projectId:string;projectName:string;createdBy:string;createdAt:number;runs:TaskRun[];revision:number;updatedAt:number}
export interface Receipt{taskId:string;runId:string;revision:number;planRevision:number;criterionIds:string[];status:RunStatus;createdBy:string}
export const currentRun=(task:Task)=>task.runs.at(-1)!;
export const taskStatus=(task:Task)=>statuses[currentRun(task).status];
export const taskRequest=<T=unknown>(op:string,args:Record<string,unknown>={})=>invoke<T>('task_ledger_request',{request:{op,...args}});
export function latestResult(run:TaskRun,criterion:Criterion,stage:Scope):{result:TestResult;evidence?:string;actor?:string;agent?:string}{
  const matching=run.validationPlan.records.filter(v=>v.planRevision===run.validationPlan.revision&&v.stage===stage),latest=matching.at(-1);
  if(latest)for(const record of matching.filter(v=>v.target===latest.target&&v.environment===latest.environment).reverse()){const result=record.results.find(r=>r.criterionId===criterion.id);if(result)return {...result,actor:record.actor,agent:record.agent};}
  return {result:'not_tested'};
}
export function acceptanceReady(run:TaskRun){const items=run.validationPlan.items.filter(c=>c.required);return items.length>0&&items.every(c=>{const result=latestResult(run,c,c.scope==='local'?'local':'release');return result.actor==='human'&&result.result==='passed';});}
export function stageProgress(run:TaskRun,stage:'local'|'release'){const items=run.validationPlan.items.filter(c=>c.scope==='both'||c.scope===stage),checks=items.map(c=>latestResult(run,c,stage));return {total:items.length,passed:checks.filter(c=>c.result==='passed').length,failed:checks.filter(c=>c.result==='failed').length};}
export function filterTasks(tasks:Task[],query:string,project:string,status:string){const q=query.trim().toLocaleLowerCase();return tasks.filter(t=>(!project||t.projectId===project)&&(status==='all'||(status==='open'?currentRun(t).status!=='closed':currentRun(t).status===status))&&(!q||[t.title,t.description,...t.runs.flatMap(r=>[r.title,r.goal,r.solution,r.description,...r.timeline.map(e=>e.summary)])].join(' ').toLocaleLowerCase().includes(q)));}
export const actorLabel=(source:string)=>source==='human'?'人工':source==='codex'?'Codex':source==='unknown'||source==='agent'?'历史 Agent':source;
export const fileLabels:Record<string,string>={added:'新增',modified:'修改',deleted:'删除',renamed:'重命名',conflicted:'冲突',untracked:'未跟踪'};

export function criterionCheck(run:TaskRun,criterion:Criterion){
  const stage=criterion.scope==='local'?'local':'release';
  const check=latestResult(run,criterion,stage);
  return {...check,stage,passed:check.actor==='human'&&check.result==='passed'};
}
export function criterionRecords(run:TaskRun,criterionId:string){
  return run.validationPlan.records.filter(v=>v.results.some(r=>r.criterionId===criterionId)).reverse();
}
export function verificationContext(run:TaskRun,stage:Scope){
  const previous=run.validationPlan.records.filter(v=>v.planRevision===run.validationPlan.revision&&v.stage===stage).at(-1);
  return {target:previous?.target??'',environment:previous?.environment??''};
}
export const formatTaskTime=(n:number)=>new Date(n).toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false});
export interface AgentSetup{config:string;instructions:string;defaultInstructions:string;instructionsRevision:number;instructionsCustomized:boolean}
