import {describe, expect, it} from 'vitest';
import {agentInstallPrompt, taskMemoryGuidance} from './taskAgentInstall';
import type {AgentClientSetup} from './taskLedger';
const client:AgentClientSetup={id:'codex',name:'Codex',config:'[mcp_servers.codepet_tasks]\ncommand = "/Users/test/中文 Space/mcp"',configPath:'~/.codex/config.toml',rulePath:'~/.codex/AGENTS.md'};
describe('Agent installation handoff',()=>{
 it('separates durable invocation policy from the exact server guidance and client config',()=>{
  const guidance='实施前登记。\n保留验收清单。';const text=agentInstallPrompt(client,guidance);
  expect(text).toContain('Task Memory MCP');
  expect(text).toContain(client.config);expect(text).toContain(client.configPath);expect(text).toContain(client.rulePath);expect(text).toContain(guidance);
  expect(text).toContain('保留其他服务器和已有设置');expect(text).toContain('不要修改内置 system prompt');
  const durable=text.match(/<!-- codepet-task-memory:start -->([\s\S]*?)<!-- codepet-task-memory:end -->/)![1];
  expect(durable).toContain('find relevant prior runs');expect(durable).not.toContain(guidance);
  expect(text).toContain('不要把整段 MCP 使用说明复制进去');
  expect(text).toContain('只读查询');expect(text).toContain('不要创建测试任务');
 });
 it('can install only the MCP connection without writing durable rules',()=>{
  const text=agentInstallPrompt(client,'服务说明',{persistGuidance:false});
  expect(text).toContain(client.config);expect(text).toContain(client.configPath);
  expect(text).not.toContain(client.rulePath);expect(text).not.toContain(taskMemoryGuidance);
  expect(text).toContain('本次只配置 MCP 连接');expect(text).toContain('保留现有持久化规则');
 });
 it('keeps an unsaved server draft explicit rather than claiming it is installed',()=>{
  const text=agentInstallPrompt(client,'仅存在于编辑器的草稿',{instructionsSaved:false});
  expect(text).toContain('仅存在于编辑器的草稿');expect(text).toContain('含未保存草稿');
  expect(text).toContain('不要宣称草稿已在服务器生效');
 });
 it('keeps code fences in custom guidance inside the copied text block',()=>{
  const guidance='规则\n```text\n示例\n```';
  expect(agentInstallPrompt(client,guidance)).toContain('````text\n'+guidance+'\n````');
 });
 it('rejects empty guidance rather than copying an incomplete installation request',()=>{
  expect(()=>agentInstallPrompt(client,' \n')).toThrow('MCP 使用说明不能为空');
 });
});
