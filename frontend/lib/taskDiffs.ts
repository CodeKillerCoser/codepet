export interface DiffFile {
  path:string;originalPath:string|null;changeType:string;
  added:number|null;deleted:number|null;error:string|null;
}
export interface DiffVersion {id:string;commit:string|null;capturedAt:number;observedAt:number;files:DiffFile[];error:string|null}
export interface FileDiff extends DiffFile {patch:string}
export interface DiffLine {kind:'hunk'|'context'|'added'|'deleted'|'note';old:number|null;next:number|null;text:string}
/** Git unified hunks, including multiple renamed-file sections and missing final newlines. */
export function patchLines(patch:string):DiffLine[]{
  const result:DiffLine[]=[];let old=0,next=0,inHunk=false;
  for(const line of patch.split('\n')){
    if(line.startsWith('diff --git ')){inHunk=false;continue;}
    const hunk=/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if(hunk){old=Number(hunk[1]);next=Number(hunk[2]);inHunk=true;result.push({kind:'hunk',old:null,next:null,text:line});continue;}
    if(!inHunk)continue;
    if(line[0]==='+')result.push({kind:'added',old:null,next:next++,text:line.slice(1)});
    else if(line[0]==='-')result.push({kind:'deleted',old:old++,next:null,text:line.slice(1)});
    else if(line[0]===' ')result.push({kind:'context',old:old++,next:next++,text:line.slice(1)});
    else if(line[0]==='\\')result.push({kind:'note',old:null,next:null,text:line.slice(1).trim()});
  }
  return result;
}
