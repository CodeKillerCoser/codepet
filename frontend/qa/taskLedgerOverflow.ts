// Explicit layout fixtures: long real-world shapes, never production writes.
import {currentRun, type Task} from '../lib/taskLedger';
export function taskLedgerOverflow(base: Task): Task[] {
  const titles = [
    '调整任务执行详情并支持编辑 Agent 接入提示词',
    '统一全局侧栏卡片、动画与弹窗层级',
    '实现 Task / TaskRun 与四工具 Agent MCP',
    '收紧任务页布局并统一内容区二级导航',
    '任务 MCP 改为本机 HTTP',
    '保留 Codex 工作台代码状态扫描结果'
  ];
  return titles.map((title, index) => {
    const task = structuredClone(base);
    task.id = `overflow-task-${index}`;
    task.title = title;
    task.description = '按确认的视觉稿展示运行卡片、目标与方案、工作区、逐项验收和活动；接入提示词可编辑并实际由 MCP 使用。';
    task.updatedAt -= index * 3600000;
    task.runs = [structuredClone(currentRun(task))];
    const run = currentRun(task);
    run.id = `overflow-run-${index}`;
    run.title = '任务执行详情与提示词编辑完成，包含本次完整处理结论与实现方案';
    run.goal = '按确认视觉稿展示任务执行，并允许用户编辑 Agent 接入提示词';
    run.solution = '执行增加 title/kind/goal/solution，旧 v3/v2 原子导入 v4；详情使用低对比运行卡片及四区，验收逐项追加结果；提示词由 Rust 原子保存并在 HTTP/stdio 新初始化返回。';
    run.agentWorkspace.path = '/Users/wangxin/.codex/worktrees/a1e0/codepet/' + 'long-workspace-name-'.repeat(12);
    run.agentWorkspace.branch = 'codex/' + 'long-branch-name-'.repeat(12);
    run.agentWorkspace.relatedCommits = ['1234567890abcdef'.repeat(3)];
    run.agentWorkspace.changedFiles = [{path: 'frontend/' + 'long-file-name-'.repeat(12) + '.svelte', changeType: 'modified', staged: false, originalPath: null}];
    return task;
  });
}
