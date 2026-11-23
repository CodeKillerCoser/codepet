import { invoke } from '@tauri-apps/api/core';

export type Phase = 'in_progress' | 'ready_for_delivery' | 'published';
export type TaskStatus = Phase | 'closed';
export type Scope = 'local' | 'release' | 'both';
export type TestResult = 'passed' | 'failed' | 'not_tested' | 'blocked';
export const phases: Record<Phase, string> = { in_progress:'进行中', ready_for_delivery:'待交付', published:'已发布' };
export const statuses: Record<TaskStatus,string> = {...phases,closed:'已结束'};
export const results: Record<TestResult, string> = {passed:'通过',failed:'失败',not_tested:'未测',blocked:'受阻'};
export const scopes: Record<Scope, string> = {local:'本地',release:'发布后',both:'本地与发布后'};
export const roundStatus = (round:Round):TaskStatus => round.closure?'closed':round.phase;
export interface Criterion { id:string; title:string; precondition:string; steps:string; expected:string; scope:Scope; required:boolean }
export interface PlanInput { summary:string; affectedAreas:string[]; criteria:Criterion[] }
export interface Plan extends PlanInput { revision:number; createdAt:number }
export interface CheckResult { criterionId:string; result:TestResult; evidence:string }
export interface VerificationInput { planRevision:number; stage:Scope; target:string; environment:string; results:CheckResult[] }
export interface CodeObservation { workspace:string; branch:string|null; head:string|null; dirty:boolean|null; upstream:string|null; commits:{sha:string;inCachedUpstream:boolean|null}[]; observedAt:number; error:string|null }
export interface DeliveryFacts { summary:string; note:string; items:{label:string;scope:'shared_worktree'|'linked_commits';observedAt:number}[] }
export interface Verification extends VerificationInput { id:string; actor:string; createdAt:number; workspaceObservation:CodeObservation|null }
export interface Closure { outcome:string; reason:string; actor:string; closedAt:number; verificationIds:string[] }
export interface TaskEvent { id:string; at:number; actor:string; kind:string; summary:string }
export interface Round { id:string; number:number; openedAt:number; openedReason:string; previousRoundId:string|null; objective:string; description:string; phase:Phase; sources:{threadId:string;verified:boolean}[]; codeRefs:CodeObservation[]; deliveryFacts?:DeliveryFacts; plans:Plan[]; verifications:Verification[]; deliveries:string[]; closure:Closure|null; events:TaskEvent[] }
export interface Task { schemaVersion:number; id:string; projectId:string; projectName:string; repositoryPath:string; workspace:string; title:string; objective:string; description:string; kind:string; currentRoundId:string; rounds:Round[]; revision:number; createdAt:number; updatedAt:number }
export interface SaveInput { taskId?:string; expectedRevision?:number; operationId?:string; workspace?:string; title?:string; objective?:string; description?:string; kind?:string; threadId?:string; phase?:Phase; note?:string; commits?:string[]; plan?:PlanInput; verification?:VerificationInput; reopenReason?:string; delivery?:string }
export interface Receipt { taskId:string; roundId:string; revision:number; planRevision:number|null; criterionIds:string[]; sourceLinked:boolean; status?:TaskStatus|null }
export const taskRequest = <T>(op:string,args:Record<string,unknown>={}) => invoke<T>('task_ledger_request',{request:{op,...args}});
export const currentRound = (task:Task) => task.rounds[task.rounds.length-1];
export const taskStatus = (task:Task) => statuses[roundStatus(currentRound(task))];

// Only compare results for the latest plan and actual target/environment of the selected stage.
export function latestResult(round:Round, criterion:Criterion, stage:Scope):{result:TestResult; actor?:string; evidence?:string} {
  const revision=round.plans.at(-1)?.revision;
  const runs=round.verifications.filter(v=>v.planRevision===revision&&v.stage===stage);
  const latest=runs.at(-1);
  if(!latest) return {result:'not_tested'};
  for(const run of [...runs].reverse()) {
    if(run.target!==latest.target||run.environment!==latest.environment) continue;
    const result=run.results.find(r=>r.criterionId===criterion.id);
    if(result) return {...result,actor:run.actor};
  }
  return {result:'not_tested'};
}
export function acceptanceReady(round:Round) {
  const required=round.plans.at(-1)?.criteria.filter(c=>c.required)??[];
  return required.length>0&&required.every(c=>{const r=latestResult(round,c,c.scope==='local'?'local':'release');return r.actor==='human'&&r.result==='passed';});
}
export function filterTasks(tasks:Task[],query:string,project:string,status:string) {
  const q=query.trim().toLocaleLowerCase();
  return tasks.filter(t=>(!project||t.projectId===project)&&(status==='all'||(status==='open'?!currentRound(t).closure:status==='closed'?!!currentRound(t).closure:roundStatus(currentRound(t))===status))&&(!q||[t.title,t.objective,t.description,...t.rounds.map(r=>r.openedReason)].join(' ').toLocaleLowerCase().includes(q)));
}
export function verificationSummary(round:Round) {
  const criteria=round.plans.at(-1)?.criteria??[];
  if(!criteria.length)return '尚无验收计划';
  const counts=criteria.map(c=>latestResult(round,c,c.scope==='local'?'local':'release').result);
  return `${counts.filter(r=>r==='passed').length}/${criteria.length} 项通过${counts.includes('failed')?' · 有失败项':''}`;
}

export function stageProgress(round:Round, stage:'local'|'release') {
  const criteria=round.plans.at(-1)?.criteria.filter(c=>c.scope==='both'||c.scope===stage)??[];
  const checks=criteria.map(c=>latestResult(round,c,stage));
  return {total:criteria.length,passed:checks.filter(r=>r.result==='passed').length,failed:checks.filter(r=>r.result==='failed').length};
}
