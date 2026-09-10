<script lang="ts">
  import { tick } from "svelte";
  import type { LineageMessage } from "./taskLineage";
  export let messages: LineageMessage[] = [];
  export let title = "原始消息";
  export let onSend: ((text: string) => Promise<void>) | null = null;
  export let compact = false;
  export let busy = false;
  export let contextId = "";
  export let anchorEventId = "";
  let text = "", sending = false, error = "";
  let limit = 50, previousContext = "", appliedAnchor = "", messageElement: HTMLDivElement;
  const drafts = new Map<string, string>();
  $: if (contextId !== previousContext) { drafts.set(previousContext, text); previousContext = contextId; text = drafts.get(contextId) ?? ""; error = ""; limit = 50; appliedAnchor = ""; }
  $: visibleMessages = messages.slice(-limit);
  $: if (anchorEventId && anchorEventId !== appliedAnchor && messages.some(m => m.evidence.eventId === anchorEventId)) {
    const index = messages.findIndex(m => m.evidence.eventId === anchorEventId); limit = Math.max(limit, messages.length - index); appliedAnchor = anchorEventId;
    void tick().then(() => messageElement?.querySelector(`[data-event-id="${CSS.escape(anchorEventId)}"]`)?.scrollIntoView({ block: "nearest" }));
  }
  async function earlier() { const height = messageElement.scrollHeight; limit += 50; await tick(); messageElement.scrollTop += messageElement.scrollHeight - height; }
  async function send() {
    if (!text.trim() || !onSend || sending) return;
    sending = true; error = "";
    const sentContext = contextId;
    try { await onSend(text.trim()); drafts.delete(sentContext); if (contextId === sentContext) text = ""; } catch (e) { if (contextId === sentContext) error = String(e); }
    finally { sending = false; }
  }
</script>
<section class="message-panel" class:compact aria-label={title}>
  <div class="messages" aria-busy={busy} bind:this={messageElement}>
    {#if messages.length > limit}<button on:click={earlier}>加载更早的消息（剩余 {messages.length - limit} 条）</button>{/if}
    {#if !messages.length}<p class="empty">{busy ? "正在读取消息…" : "暂无可显示的原始消息"}</p>{/if}
    {#each visibleMessages as message (message.evidence.eventId)}
      <article data-event-id={message.evidence.eventId}>
        <header><strong>{message.role === "user" ? "你" : "Codex"}</strong><time>{message.timestamp ? new Date(message.timestamp).toLocaleString() : ""}</time></header>
        <p>{message.text}</p>
        <details><summary>查看证据位置</summary><small>{message.evidence.file}<br />字节位置 {message.evidence.byteOffset} · 版本 {message.evidence.generation}</small></details>
      </article>
    {/each}
  </div>
  {#if onSend}
    <form on:submit|preventDefault={send}>
      <label>继续当前线程<textarea bind:value={text} rows={compact ? 2 : 3} disabled={sending} placeholder="要求验收、修正或继续任务…"></textarea></label>
      <div class="send-row">{#if error}<p role="alert">{error}</p>{/if}<button disabled={sending || busy || !text.trim()} type="submit">{sending ? "发送中…" : "发送消息"}</button></div>
    </form>
  {/if}
</section>
<style>
  .message-panel{display:flex;flex-direction:column;min-height:0;height:100%;overflow:hidden}.messages{flex:1;overflow:auto;padding:18px;min-height:160px;overscroll-behavior:contain}article{margin:0 0 24px}header{display:flex;justify-content:space-between;gap:10px;font-size:12px}time,.empty,small{color:var(--color-text-secondary,var(--app-muted))}p{white-space:pre-wrap;overflow-wrap:anywhere;font-size:13px;line-height:1.7}details{font-size:11px}small{overflow-wrap:anywhere}form{padding:14px;border-top:1px solid var(--color-main-divider);flex:none}label{font-size:12px}textarea{display:block;width:100%;resize:vertical;min-height:58px;max-height:200px;margin-top:7px}.send-row{display:flex;justify-content:flex-end;gap:8px;margin-top:8px}.send-row p{color:var(--color-main-danger-text);margin:0;font-size:12px}button{white-space:nowrap}.compact .messages{padding:12px}.compact article{margin-bottom:18px}
</style>
