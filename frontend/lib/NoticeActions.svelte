<script lang="ts">
  import { tick } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { noticeActionExecute, type Notice, type NoticeAction, type NoticeActionInput } from './noticeGateway';
  export let notice: Notice;
  export let available = true;
  let activeId = '', text = '', result = '', busy = false, blocked = false, revision = 0;
  let input: HTMLTextAreaElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  let submitted = new Set<string>();
  const actionKey = (id: string) => `${notice.noticeId}:${notice.revision}:${id}`;
  $: actions = notice.actions.filter(action => action.enabled && ['reply','permission'].includes(action.kind));
  $: selected = actions.find(action => action.id === activeId);
  async function open(action: NoticeAction, event: MouseEvent) {
    event.stopPropagation(); if (busy || submitted.has(actionKey(action.id))) return; trigger = event.currentTarget as HTMLButtonElement;
    activeId = action.id; text = ''; result = ''; blocked = false; revision = notice.revision;
    await tick();
    if ('__TAURI_INTERNALS__' in window) {
      try { await getCurrentWindow().setFocus(); } catch (error) { result = `无法聚焦通知输入：${String(error)}`; }
    }
    await tick(); input?.focus();
  }
  async function close() { if (busy) return; activeId = ''; await tick(); trigger?.focus(); }
  async function execute(value: NoticeActionInput) {
    if (busy || blocked || !selected || !available) return; busy = true; result = '';
    try { const response = await noticeActionExecute(notice.noticeId, selected.id, revision, crypto.randomUUID(), value);
      blocked = response.status === 'unknown' || response.status === 'accepted' || response.status === 'succeeded';
      result = response.message || ({ accepted: '已受理', succeeded: '操作成功', failed: '操作失败', unknown: '结果待确认，请勿重复提交' }[response.status] ?? '结果待确认');
    } catch (error) { blocked = true; result = `${String(error)}；请确认结果后再操作`; }
    finally { if (blocked) submitted = new Set([...submitted, `${notice.noticeId}:${revision}:${activeId}`]); busy = false; }
  }
</script>
{#if actions.length && available}
  <div class="notice-actions" role="group" aria-label="通知操作">
    <div class="action-buttons">{#each actions as action (action.id)}<button type="button" disabled={busy || submitted.has(actionKey(action.id))} on:click={event => open(action,event)}>{action.label}</button>{/each}</div>
    {#if selected?.kind === 'reply'}
      <form on:submit|preventDefault={() => execute({kind:'reply',text})}>
        <label>回复 {notice.content.title}<textarea bind:this={input} bind:value={text} placeholder={selected.placeholder ?? '输入回复'} rows="2" disabled={busy || blocked} on:keydown={event => { if(event.key === 'Escape') void close(); }}></textarea></label>
        <button type="submit" disabled={busy || blocked || !text.trim() || !available}>发送</button><button type="button" disabled={busy} on:click={close}>关闭</button>
      </form>
    {:else if selected?.kind === 'permission'}
      <div class="permission-options"><p>{selected.description}</p>{#each selected.options ?? [] as option (option.id)}<button type="button" disabled={busy || blocked} on:click={() => execute({kind:'permission',optionId:option.id})}>{option.label}</button>{/each}<button type="button" disabled={busy} on:click={close}>关闭</button></div>
    {/if}
    {#if result}<p class="action-result" role="status">{result}</p>{/if}
  </div>
{/if}
<style>
  .notice-actions { margin-top:8px; min-width:0; color:var(--app-text); }
  .action-buttons,.permission-options { display:flex; flex-wrap:wrap; gap:6px; }
  button { padding:4px 8px; border:1px solid var(--app-border); border-radius:5px; background:var(--app-surface); color:inherit; font:inherit; cursor:pointer; }
  button:disabled { opacity:.5; cursor:default; }
  button:focus-visible,textarea:focus-visible { outline:2px solid var(--app-accent); outline-offset:2px; }
  label { display:block; font-size:12px; } textarea { display:block; box-sizing:border-box; width:100%; min-width:0; resize:vertical; margin:6px 0; background:var(--app-surface); color:inherit; border:1px solid var(--app-border); border-radius:4px; padding:6px; font:inherit; }
  p { margin:5px 0; font-size:12px; overflow-wrap:anywhere; }.permission-options p { width:100%; }
</style>
