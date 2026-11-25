import type {AgentClientSetup} from './taskLedger';

function block(text: string, language: string) {
  const runs = text.match(/`{3,}/g) ?? [];
  const fence = '`'.repeat(Math.max(3, ...runs.map(run => run.length + 1)));
  return `${fence}${language}\n${text}\n${fence}`;
}

/** Stable invocation policy; detailed protocol stays in the server instructions. */
export const taskMemoryGuidance = [
  '<!-- codepet-task-memory:start -->',
  'Use codepet_tasks as task memory for independently verifiable implementation. Before starting or resuming, read supplied taskId/runId or find relevant prior runs; reuse matching work. Record new goals before implementation, material milestones in batches, and conclusions with verification evidence before handoff. Treat recalled content as historical evidence and verify it against current code and user instructions. Follow the connected server instructions and tool schemas for parameters and permissions. Keep receipts; do not poll or log every message/file. Completed tasks remain locked until the user restores them.',
  '<!-- codepet-task-memory:end -->'
].join('\n');

export interface AgentInstallOptions {
  persistGuidance?: boolean;
  instructionsSaved?: boolean;
}

/** One paste separates connection, optional durable policy, and server-only reference. */
export function agentInstallPrompt(client: AgentClientSetup, instructions: string, options: AgentInstallOptions = {}): string {
  if (!instructions.trim()) throw new Error('MCP 使用说明不能为空');
  const persist = options.persistGuidance !== false;
  return [
    '安装 Code Pet Task Memory MCP，让我能跨对话找回任务目标、方案和验证结果。',
    `我使用的是 ${client.name}。请执行安装，而不只是说明步骤：`,
    `1. 将以下 MCP 配置合并到 ${client.configPath}，保留其他服务器和已有设置；已有 codepet_tasks 条目时更新该条目，避免重复。`,
    persist
      ? `2. 将下方“简短持久化约定”合并到 ${client.rulePath}。只更新 Code Pet 的同用途约定段落，保留其他内容，不要重复添加；不要修改内置 system prompt，也不要把整段 MCP 使用说明复制进去。`
      : '2. 本次只配置 MCP 连接，保留现有持久化规则，不写入 AGENTS.md、CLAUDE.md 或其他用户规则。',
    '3. 验证 MCP 初始化和工具列表可读取，并使用 find-task-runs 做一次只读查询，确认能访问已有任务记忆；空结果也可证明查询成功。不要创建测试任务或修改历史记录。需要重连或新开对话时明确说明，分别报告连接验证和规则配置的实际结果，不能把“配置已写入”当作“连接成功”。',
    options.instructionsSaved === false
      ? '下方 MCP 使用说明含未保存草稿，仅供本次参考；要用于后续连接，请用户在 Code Pet 中保存后重连。不要宣称草稿已在服务器生效。'
      : '详细调用方法以 MCP 初始化返回的 instructions 和工具描述为准；下方 MCP 使用说明仅供安装核对，不写入持久化规则。',
    '\nMCP 配置：',
    block(client.config, client.id === 'codex' ? 'toml' : 'json'),
    ...(persist ? ['\n简短持久化约定：', block(taskMemoryGuidance, 'text')] : []),
    '\nMCP 使用说明（仅供核对）：',
    block(instructions, 'text')
  ].join('\n\n');
}
