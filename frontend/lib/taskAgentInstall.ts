import type {AgentClientSetup} from './taskLedger';

function block(text: string, language: string) {
  const runs = text.match(/`{3,}/g) ?? [];
  const fence = '`'.repeat(Math.max(3, ...runs.map(run => run.length + 1)));
  return `${fence}${language}\n${text}\n${fence}`;
}

/** One paste carries both client-specific setup and the currently edited guidance. */
export function agentInstallPrompt(client: AgentClientSetup, instructions: string): string {
  if (!instructions.trim()) throw new Error('提示词不能为空');
  return [
    '安装这个 MCP，并将下面的使用约定补充到系统提示词里。',
    `我使用的是 ${client.name}。请执行安装，而不只是说明步骤：`,
    `1. 将以下 MCP 配置合并到 ${client.configPath}，保留其他服务器和已有设置；已有 codepet_tasks 条目时更新该条目，避免重复。`,
    `2. 将使用约定补充到 ${client.rulePath}。这里指该 Agent 支持的持久化全局指令或用户规则；保留已有内容，不要替换内置系统提示词。同一约定不要重复添加。`,
    '3. 检查配置并验证 MCP 能否连接；如果需要重连或新开对话，请明确告诉我。不能自动完成的步骤请说明，不要宣称已经安装成功。',
    '\nMCP 配置：',
    block(client.config, client.id === 'codex' ? 'toml' : 'json'),
    '\n使用约定：',
    block(instructions, 'text')
  ].join('\n\n');
}
