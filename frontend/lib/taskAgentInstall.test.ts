import {describe, expect, it} from 'vitest';
import {agentInstallPrompt} from './taskAgentInstall';
import type {AgentClientSetup} from './taskLedger';
const client:AgentClientSetup={id:'codex',name:'Codex',config:'[mcp_servers.codepet_tasks]\ncommand = "/Users/test/中文 Space/mcp"',configPath:'~/.codex/config.toml',rulePath:'~/.codex/AGENTS.md'};
describe('Agent installation handoff',()=>{
 it('carries the exact client config, edited guidance and installation destinations in one paste',()=>{
  const guidance='实施前登记。\n保留验收清单。';const text=agentInstallPrompt(client,guidance);
  expect(text).toContain('安装这个 MCP');expect(text).toContain('系统提示词');
  expect(text).toContain(client.config);expect(text).toContain(client.configPath);expect(text).toContain(client.rulePath);expect(text).toContain(guidance);
  expect(text).toContain('保留其他服务器和已有设置');expect(text).toContain('不要替换内置系统提示词');
 });
 it('keeps code fences in custom guidance inside the copied text block',()=>{
  const guidance='规则\n```text\n示例\n```';
  expect(agentInstallPrompt(client,guidance)).toContain('````text\n'+guidance+'\n````');
 });
 it('rejects empty guidance rather than copying an incomplete installation request',()=>{
  expect(()=>agentInstallPrompt(client,' \n')).toThrow('提示词不能为空');
 });
});
