import { describe, it, expect } from 'vitest';
import { filterRows, statusMatches, branchLabel, changeLabel, gitSummary, type Worktree } from './codexWorkbench';
const row = (id: string, overrides: Partial<Worktree['git']> = {}): Worktree => ({id, projectId: id, projectName: id, path: `/repo/${id}`, main: true, conversations: [{id:'c', source:'Codex', title:'history', cwd:'/repo', updatedAt:new Date('2026-11-23T15:00:00+08:00').getTime(), archived:false}], git:{changes:[],upstream:false,ahead:0,behind:0,...overrides}});
const filters = { query:'', projects:[], related:true, period:'all', date:'' };
describe('history workbench filters', () => {
  it('combines project, title, activity and association filters', () => {
    const a = row('a'), b = row('b'); b.conversations = [];
    expect(filterRows([a,b], {...filters,query:'HISTORY',projects:['a'],period:'date',date:'2026-11-23'})).toEqual([a]);
    expect(filterRows([a,b], {...filters,period:'date',date:'2026-11-24'})).toEqual([]);
    expect(filterRows([b], {...filters,related:false})).toEqual([b]);
  });
  it('allows dirty and unpushed together, excludes unreadable trees from clean totals', () => {
    const r = row('a',{upstream:true,ahead:2,changes:[{id:'f',path:'f',status:'M.',staged:true}]});
    expect(statusMatches(r,'dirty')).toBe(true);expect(statusMatches(r,'unpushed')).toBe(true);
    expect(statusMatches(row('b',{error:'missing'}),'clean')).toBe(false);
    expect(statusMatches(row('c',{upstream:true,behind:2}),'pushed')).toBe(true);
  });
  it('orders conflicts, dirty and ahead before recent activity', () => {
    const clean=row('clean'),ahead=row('ahead',{upstream:true,ahead:1}),dirty=row('dirty',{changes:[{id:'x',path:'x',status:'??',staged:false}]}),conflict=row('conflict',{changes:[{id:'u',path:'u',status:'UU',staged:true,conflict:true}]});
    expect(filterRows([clean,ahead,dirty,conflict],filters).map(r=>r.id)).toEqual(['conflict','dirty','ahead','clean']);
  });
});

describe('workbench explanations reflect Git facts', () => {
  it('distinguishes no upstream, ahead/behind and unreadable directories', () => {
    expect(gitSummary(row('clean'))).toContain('未设置上游');
    const divergent=row('branch',{upstream:true,ahead:2,behind:3});
    expect(gitSummary(divergent)).toContain('2 个提交待推送');expect(gitSummary(divergent)).toContain('落后 3 个提交');
    expect(gitSummary(row('gone',{error:'directory missing'}))).toBe('读取失败');
  });
  it('names detached heads and changes without treating them as task progress', () => {
    expect(branchLabel(row('detached',{branch:'(detached)',head:'123456789'}))).toBe('游离 HEAD · 1234567');
    expect(changeLabel({id:'u',path:'u',status:'UU',staged:true,conflict:true})).toBe('冲突');
    expect(changeLabel({id:'r',path:'new',original:'old',status:'R.',staged:true})).toBe('重命名');
    expect(changeLabel({id:'n',path:'new',status:'??',staged:false})).toBe('未跟踪');
  });
});
