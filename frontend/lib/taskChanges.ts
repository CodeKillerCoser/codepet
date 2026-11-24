import type {AgentWorkspace} from './taskLedger';
export type ChangedFile = AgentWorkspace['changedFiles'][number];
export type CommitChange = NonNullable<AgentWorkspace['commitChanges']>[number];
export type FileKind = 'added'|'modified'|'deleted'|'renamed'|'conflicted'|'unknown';
export interface RunChange extends Omit<ChangedFile,'changeType'> {changeType:FileKind}
export const changeKinds:Record<FileKind,{label:string;symbol:string}>={
  added:{label:'新增',symbol:'A'},modified:{label:'修改',symbol:'M'},deleted:{label:'删除',symbol:'D'},
  renamed:{label:'重命名',symbol:'R'},conflicted:{label:'冲突',symbol:'U'},unknown:{label:'未确认',symbol:'?'}
};
export function projectRunChanges(workspace:AgentWorkspace,commits:CommitChange[]=[]){
  const normalize=(file:ChangedFile):RunChange=>{const type=file.changeType==='untracked'?'added':file.changeType;return {...file,changeType:type in changeKinds?type as FileKind:'unknown'};};
  const known=new Map<string,RunChange>();
  for(const file of [...commits.flatMap(c=>c.files),...workspace.changedFiles])known.set(file.path,normalize(file));
  const scope=workspace.runFiles!=null?'run':workspace.relatedCommits.length?'commits':'workspace';
  let files:RunChange[];
  if(workspace.runFiles!=null)files=workspace.runFiles.map(path=>known.get(path)??{path,changeType:'unknown',staged:false,originalPath:null});
  else if(scope==='commits'){
    const linked=new Map<string,RunChange>();
    for(const commit of commits)for(const file of commit.files)linked.set(file.path,normalize(file));
    files=[...linked.values()];
  }else files=[...known.values()];
  files.sort((a,b)=>a.path.localeCompare(b.path));
  const counts:Record<FileKind,number>={added:0,modified:0,deleted:0,renamed:0,conflicted:0,unknown:0};
  for(const file of files)counts[file.changeType]++;
  const complete=scope==='run'?true:scope==='commits'?workspace.relatedCommits.every(sha=>commits.some(c=>c.commit===sha&&!c.error)):workspace.changeStatus.hasUncommittedChanges!==null&&!workspace.error;
  return {scope,files,counts,complete};
}
export type RunChanges=ReturnType<typeof projectRunChanges>;

export function workspaceCommitState(workspace:AgentWorkspace){
 const {hasUncommittedChanges:dirty,hasUnpushedCommits:unpushed}=workspace.changeStatus;
 if(dirty)return {label:unpushed?'待提交 · 待推送':'待提交',state:'dirty'};
 if(dirty===null)return {label:'提交状态未知',state:'unknown'};
 if(unpushed)return {label:'待推送',state:'unpushed'};
 if(unpushed===false)return {label:'已推送',state:'synced'};
 return {label:'已提交 · 推送未知',state:'unknown'};
}
