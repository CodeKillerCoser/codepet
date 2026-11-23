import { invoke } from '@tauri-apps/api/core';
export interface Project { id: string; name: string; roots: string[] }
export interface Chat { id: string; source: string; title: string; cwd: string; updatedAt: number; archived: boolean }
export interface Change { id: string; path: string; original?: string; status: string; staged: boolean; conflict?: boolean }
export interface Worktree { id: string; projectId: string; projectName: string; path: string; main: boolean; conversations: Chat[]; git: { branch?: string; head?: string; upstream: boolean; ahead: number; behind: number; changes: Change[]; error?: string } }
export interface SourceInfo { codexHome: string; projectCatalog: "database" | "globalState" }
export interface Snapshot { sourceInfo?: SourceInfo; scanId: number; projects: Project[]; worktrees: Worktree[]; warnings: string[]; scannedAt: number }
export const workbenchRequest = <T>(op: string, args: Record<string, unknown> = {}) => invoke<T>('codex_workbench_request', { request: { op, ...args } });
export const activity = (row: Worktree) => Math.max(0, ...row.conversations.map(c => c.updatedAt));
export const dirty = (row: Worktree) => row.git.changes.length > 0;
export const unpushed = (row: Worktree) => row.git.upstream && row.git.ahead > 0;
export function statusMatches(row: Worktree, status: string) {
  if (status === 'dirty') return !row.git.error && dirty(row);
  if (status === 'unpushed') return !row.git.error && unpushed(row);
  if (status === 'pushed') return !row.git.error && !dirty(row) && row.git.upstream && !row.git.ahead;
  if (status === 'clean') return !row.git.error && !dirty(row) && !row.git.upstream;
  if (status === 'error') return !!row.git.error;
  return true;
}
export interface Filters { query: string; projects: string[]; related: boolean; period: string; date: string }
export function filterRows(rows: Worktree[], filter: Filters, now = new Date()) {
  let since = 0;
  if (filter.period === 'date' && filter.date) since = new Date(`${filter.date}T00:00:00`).getTime();
  else if (filter.period !== 'all' && filter.period !== 'date') { const day = new Date(now); day.setHours(0, 0, 0, 0); day.setDate(day.getDate() - (Number(filter.period) - 1)); since = day.getTime(); }
  const q = filter.query.trim().toLocaleLowerCase();
  return rows.filter(r => (!filter.related || r.conversations.length > 0) && (!filter.projects.length || filter.projects.includes(r.projectId)) && activity(r) >= since && (!q || [r.projectName, r.path, r.git.branch, ...r.conversations.map(c => c.title)].join(' ').toLocaleLowerCase().includes(q)))
    .sort((a, b) => Number(b.git.changes.some(c => c.conflict)) - Number(a.git.changes.some(c => c.conflict)) || Number(dirty(b)) - Number(dirty(a)) || Number(unpushed(b)) - Number(unpushed(a)) || activity(b) - activity(a) || a.path.localeCompare(b.path));
}

export const branchLabel = (row: Worktree) => row.git.branch === '(detached)' ? `游离 HEAD · ${row.git.head?.slice(0, 7) ?? ''}` : row.git.branch || '分支不可读';
export function changeLabel(change: Change) {
  if (change.conflict) return '冲突';
  if (change.status === '??') return '未跟踪';
  if (change.status.includes('D')) return '删除';
  if (change.original) return '重命名';
  if (change.status.includes('A')) return '新增';
  return '修改';
}
export function gitSummary(row: Worktree) {
  if (row.git.error) return '读取失败';
  const parts: string[] = [];
  if (dirty(row)) parts.push(`${row.git.changes.length} 项未提交`);
  if (unpushed(row)) parts.push(`${row.git.ahead} 个提交待推送`);
  if (row.git.behind) parts.push(`落后 ${row.git.behind} 个提交`);
  if (!parts.length) parts.push(row.git.upstream ? '工作区干净 · 无待推送提交' : '工作区干净 · 未设置上游');
  return parts.join(' · ');
}
