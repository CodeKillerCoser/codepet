<script lang="ts">
  import * as Dialog from './ui/dialog/index';
  import {X} from '@lucide/svelte';
  import {taskRequest, type AgentSetup, type AgentClientSetup} from './taskLedger';
  import {agentInstallPrompt} from './taskAgentInstall';
  import codexLogo from '../assets/agents/codex.png';
  import claudeLogo from '../assets/agents/claude.png';
  import cursorLogo from '../assets/agents/cursor.png';
  import opencodeLogo from '../assets/agents/opencode.png';
  import antigravityLogo from '../assets/agents/antigravity.png';

  export let returnFocus: HTMLElement;
  let dialog: Dialog.ModalHandle;
  let setup: AgentSetup;
  let draft = '', busy = false, error = '', message = '';
  let persistGuidance = true;
  const logos: Record<string, string> = {codex:codexLogo, claude:claudeLogo, cursor:cursorLogo, opencode:opencodeLogo, antigravity:antigravityLogo};
  $: clients = setup?.clients ?? (setup ? [{id:'codex',name:'Codex',config:setup.config,configPath:'~/.codex/config.toml',rulePath:'~/.codex/AGENTS.md'}] : []);

  export function isOpen() { return dialog?.isOpen(); }
  export async function show() {
    setup = await taskRequest<AgentSetup>('setup');
    draft = setup.instructions;
    persistGuidance = true;
    error = ''; message = '';
    dialog.showModal();
  }
  async function save(reset = false) {
    busy = true; error = ''; message = '';
    try {
      const saved = await taskRequest<Partial<AgentSetup>>('save-instructions', {
        expectedRevision: setup.instructionsRevision,
        text: reset ? null : draft
      });
      setup = {...setup, ...saved};
      draft = setup.instructions;
      message = reset ? '已恢复默认使用说明，重新连接 MCP 后生效' : '使用说明已保存，重新连接 MCP 后生效';
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function copyClient(item: AgentClientSetup) {
    error = ''; message = '';
    try {
      const instructionsSaved = draft === setup.instructions;
      await navigator.clipboard.writeText(agentInstallPrompt(item, draft, {persistGuidance, instructionsSaved}));
      message = instructionsSaved ? '已复制安装指令' : '已复制安装指令；使用说明草稿尚未保存';
    } catch { error = '复制失败，请重试。'; }
  }
</script>

<Dialog.Modal title="连接 Agent" class="ledger-dialog agent-setup-dialog" {returnFocus}
  bind:this={dialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={() => save()}>
    <fieldset class="form-controls" disabled={busy}>
      <header>
        <h3>连接 Agent</h3>
        <button type="button" aria-label="关闭接入说明" title="关闭" on:click={() => dialog.close()}><X size={18}/></button>
      </header>
      <p class="setup-intro">任务记忆：先查历史，再记录关键进展与验证结果。</p>
      <label class="checkbox" title="只写入调用时机，详细用法由 MCP 提供">
        <input type="checkbox" bind:checked={persistGuidance} />同时配置简短持久化约定
      </label>
      <div class="agent-clients" role="group" aria-label="支持的 Agent">
        {#each clients as item}
          <div class="agent-entry">
            <button type="button" aria-label={`复制 ${item.name} 安装指令`} aria-describedby={`agent-name-${item.id}`}
              disabled={!draft.trim()} on:click={() => copyClient(item)}>
              <img src={logos[item.id]} alt="" width="28" height="28"/>
            </button>
            <span class="agent-tooltip" id={`agent-name-${item.id}`} role="tooltip">{item.name}</span>
          </div>
        {/each}
      </div>
      <div class="prompt-editor">
        <label>MCP 使用说明<textarea aria-label="MCP 使用说明" aria-describedby="task-mcp-instructions-hint" rows="8" maxlength="16000" required bind:value={draft}></textarea></label>
        <p class="instructions-hint" id="task-mcp-instructions-hint">连接时由 MCP 提供；保存后重新连接生效。</p>
        <footer>
          <button type="button" class="quiet" disabled={!setup?.instructionsCustomized} on:click={() => save(true)}>恢复默认</button>
          <button class="primary" disabled={!draft.trim() || draft === setup?.instructions}>{busy ? '保存中…' : '确认'}</button>
        </footer>
      </div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if message}<p class="message" role="status">{message}</p>{/if}
    </fieldset>
  </form>
</Dialog.Modal>

<style>
  .agent-clients {display:flex;align-items:center;flex-wrap:wrap;gap:12px;margin:0 0 20px;}
  .setup-intro {margin:0 0 12px;}
  .instructions-hint {margin:6px 0 0;color:var(--task-secondary,var(--wb-muted));font:var(--font-caption);}
  .agent-entry {position:relative;}
  .agent-clients button {display:grid;place-items:center;width:40px;height:40px;min-height:40px;padding:6px;border:0;border-radius:6px;background:transparent;}
  .agent-clients button:hover:not(:disabled) {background:var(--wb-control);}
  .agent-clients img {display:block;object-fit:contain;border-radius:5px;}
  .agent-tooltip {position:absolute;top:calc(100% + 4px);left:50%;transform:translateX(-50%);z-index:3;visibility:hidden;opacity:0;pointer-events:none;white-space:nowrap;padding:4px 8px;border:1px solid var(--wb-border);border-radius:5px;background:var(--wb-panel);color:var(--wb-text);font:var(--font-caption);}
  .agent-entry:hover .agent-tooltip,.agent-entry:focus-within .agent-tooltip {visibility:visible;opacity:1;}
  .prompt-editor label {margin-top:0;}
  .prompt-editor footer {position:static;border-top:0;margin-top:12px;padding-top:0;padding-bottom:0;}
  .message,.error {margin:12px 0 0;font:var(--font-caption);overflow-wrap:anywhere;}
</style>
