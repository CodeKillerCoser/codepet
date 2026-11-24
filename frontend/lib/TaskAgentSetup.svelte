<script lang="ts">
  import * as Dialog from './ui/dialog/index';
  import {taskRequest, type AgentSetup} from './taskLedger';

  export let returnFocus: HTMLElement;
  let dialog: Dialog.ModalHandle;
  let setup: AgentSetup;
  let draft = '', busy = false, error = '', message = '';

  export function isOpen() { return dialog?.isOpen(); }
  export async function show() {
    setup = await taskRequest<AgentSetup>('setup');
    draft = setup.instructions;
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
      message = reset ? '已恢复默认提示词' : '提示词已保存，重新连接 MCP 后生效';
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function copy() {
    try { await navigator.clipboard.writeText(setup.config); message = '配置已复制'; }
    catch { message = '请选中配置文本复制'; }
  }
</script>

<Dialog.Modal title="连接 Agent 任务登记" class="ledger-dialog" {returnFocus}
  bind:this={dialog} onCancel={e => { if (busy) e.preventDefault(); }}>
  <form on:submit|preventDefault={() => save()}>
    <fieldset class="form-controls" disabled={busy}>
      <header>
        <h3>连接 Agent 任务登记</h3>
        <button type="button" aria-label="关闭接入说明" on:click={() => dialog.close()}>×</button>
      </header>
      <p>替换 config.toml 中的 codepet_tasks 条目，在新对话或重新连接 MCP 后使用。</p>
      {#if setup?.transport === 'stdio'}
        <p class="muted">固定入口位于用户目录，移动或重命名 Code Pet 不影响连接。Agent 会自动启动独立程序，无需保持 App 运行；启动新版 App 后会更新程序和使用的数据目录。</p>
      {:else}
        <p class="muted">本机 HTTP 服务随 Code Pet 后台运行。</p>
      {/if}
      <label>MCP 配置<textarea readonly rows="5" value={setup?.config ?? ''}></textarea></label>
      <button type="button" on:click={copy}>复制配置</button>
      <p class="muted">{setup?.transport === 'stdio' ? 'CODEPET_AGENT' : 'X-CodePet-Agent'} 标记调用来源，默认 codex。</p>
      <label>Agent 使用提示词
        <textarea aria-label="Agent 使用提示词" rows="10" maxlength="16000" required bind:value={draft}></textarea>
      </label>
      <p class="muted">保存后在新对话或重新连接 MCP 时提供给 Agent，已连接的会话不会自动更新。</p>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if message}<p class="message" role="status">{message}</p>{/if}
      <footer>
        <button type="button" class="quiet" disabled={!setup?.instructionsCustomized} on:click={() => save(true)}>恢复默认</button>
        <button class="primary" disabled={!draft.trim() || draft === setup?.instructions}>{busy ? '保存中…' : '保存提示词'}</button>
      </footer>
    </fieldset>
  </form>
</Dialog.Modal>
