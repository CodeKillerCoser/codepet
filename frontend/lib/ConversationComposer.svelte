<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { codepetGateway, GatewayRequestError } from './codepetGateway';
  import { conversationDrafts, conversationDraftBackups, pendingQueueSubmissions } from './conversationInputDrafts';
  import type { Conversation, GatewayCapabilities, InputQueueItem, InputQueueSnapshot, ProtocolEvent, RoutedResourceId, TurnSelection, TurnTask } from '../../sdk/typescript/codepet-gateway-sdk/src/generated';
  export let conversation: RoutedResourceId;
  export let onSend: ((text: string) => Promise<void>) | null = null;
  let key = '', text = '', error = '', notice = '', loading = false, busy = false, pending = false;
  let queue: InputQueueSnapshot | null = null, capabilities: GatewayCapabilities | null = null;
  let activeTurn: TurnTask | null = null, status: Conversation['status'] = 'idle';
  let selection: TurnSelection = {}, ready = false, epoch = 0, liveRevision = 0, fetching = false;
  let textarea: HTMLTextAreaElement, mounted = false, backupCount = 0;
  $: supportsQueue = !!capabilities?.methods.includes('conversation.inputQueue.enqueue');
  $: canSteer = ready && !!capabilities?.methods.includes('conversation.inputQueue.steer') && !!activeTurn && status === 'running';
  $: if (mounted && JSON.stringify(conversation) !== key) switchConversation();
  $: if (key) conversationDrafts.set(key, { text, selection });
  const editable = (item: InputQueueItem) => item.state === 'queued' || item.state === 'blocked';
  const same = (route: RoutedResourceId) => route.providerId === conversation.providerId && route.nativeResourceId === conversation.nativeResourceId;
  function switchConversation() {
    key = JSON.stringify(conversation); epoch++; liveRevision++;
    const draft = conversationDrafts.get(key); text = draft?.text ?? ''; selection = draft?.selection ?? {};
    backupCount = conversationDraftBackups.get(key)?.length ?? 0;
    pending = pendingQueueSubmissions.has(key); queue = null; activeTurn = null;
    capabilities = null; ready = false; error = ''; notice = ''; busy = false;
    void initialize();
  }
  async function initialize() {
    const generation = epoch, route = { ...conversation }, observedLive = liveRevision;
    loading = true;
    try {
      const [description, snapshot] = await Promise.all([
        codepetGateway('provider.describe', { providerId: route.providerId }),
        codepetGateway('conversation.get', { conversation: route, limit: 1 }),
      ]);
      if (generation !== epoch) return;
      capabilities = description.capabilities; ready = description.provider.runtime.status === 'ready';
      if (observedLive === liveRevision) { activeTurn = snapshot.conversation.activeTurn ?? null; status = snapshot.conversation.status; }
      if (!Object.keys(selection).length) selection = snapshot.conversation.selection ?? {};
      if (capabilities.methods.includes('conversation.inputQueue.get')) await refreshQueue();
    } catch (e) { if (generation === epoch) error = String(e); }
    finally { if (generation === epoch) loading = false; }
  }
  async function refreshQueue() {
    if (fetching || !capabilities?.methods.includes('conversation.inputQueue.get')) return;
    const generation = epoch, route = { ...conversation };
    fetching = true;
    try {
      const snapshot = await codepetGateway('conversation.inputQueue.get', { conversation: route });
      if (generation === epoch && (!queue || snapshot.revision >= queue.revision)) queue = snapshot;
    } catch (e) { if (generation === epoch) error = String(e); }
    finally { fetching = false; }
  }
  async function send(retry = false) {
    if (busy || !ready || (!text.trim() && !retry)) return;
    const generation = epoch, sentKey = key, sentText = text, route = { ...conversation };
    busy = true; error = ''; notice = '';
    try {
      if (supportsQueue) {
        let request = pendingQueueSubmissions.get(sentKey);
        if (request && !retry) throw new Error('请先核对上次入队结果');
        if (!request && !Object.keys(selection).length) {
          const interaction = await codepetGateway('conversation.acquireInteraction', { conversation: route });
          if (generation !== epoch) return;
          selection = interaction.selection;
        }
        request ??= { conversation: route, clientRequestId: crypto.randomUUID(), capabilityRevision: capabilities!.revision, input: { kind: 'text', text: sentText }, selection };
        pendingQueueSubmissions.set(sentKey, request); pending = true;
        await codepetGateway('conversation.inputQueue.enqueue', request);
        pendingQueueSubmissions.delete(sentKey);
        if (generation !== epoch) return;
        pending = false; if (text === request.input.text) text = '';
        await refreshQueue();
      } else if (onSend) {
        await onSend(sentText); if (generation === epoch && text === sentText) text = '';
      }
    } catch (e) {
      if (e instanceof GatewayRequestError && !['provider_timeout','transport_error'].includes(e.code)) pendingQueueSubmissions.delete(sentKey);
      if (generation === epoch) { pending = pendingQueueSubmissions.has(sentKey); error = pending ? '入队结果尚未确认，请核对原请求。' : String(e); }
    } finally { if (generation === epoch) busy = false; }
  }
  async function action(operation: () => Promise<void>) {
    if (busy) return; const generation = epoch; busy = true; error = ''; notice = '';
    try { await operation(); } catch (e) { if (generation === epoch) error = String(e); }
    finally { if (generation === epoch) { busy = false; await refreshQueue(); } }
  }
  async function restore(item: InputQueueItem) {
    const generation = epoch, route = { ...conversation };
    if (text.trim()) { const backups = conversationDraftBackups.get(key) ?? []; backups.push({ text, selection }); conversationDraftBackups.set(key, backups); backupCount = backups.length; }
    text = item.input.text; selection = item.selection; await tick();
    if (generation !== epoch) return;
    textarea?.focus(); textarea?.setSelectionRange(text.length, text.length);
    await codepetGateway('conversation.inputQueue.acknowledgeEdit', { conversation: route, itemId: item.id });
  }
  async function withdraw(item: InputQueueItem, edit: boolean) {
    const generation = epoch;
    if (item.state === 'outcomeUnknown' && !window.confirm('请先核对会话。此操作只移除待核对记录，不会撤回已发送消息或重新发送。确认已核对？')) return;
    await action(async () => {
      const response = await codepetGateway('conversation.inputQueue.withdraw', { conversation, itemId: item.id, clientRequestId: crypto.randomUUID(), expectedRevision: queue!.revision, kind: edit ? 'edit' : 'delete' });
      if (edit && generation === epoch) await restore(response.item);
    });
  }
  async function steer(item: InputQueueItem) {
    if (!activeTurn || !canSteer) return; const generation = epoch;
    await action(async () => {
      const response = await codepetGateway('conversation.inputQueue.steer', { conversation, itemId: item.id, clientRequestId: crypto.randomUUID(), expectedRevision: queue!.revision, turn: activeTurn!.resource, capabilityRevision: capabilities!.revision });
      if (generation === epoch && response.item.state === 'accepted') notice = '引导已受理，等待消息同步；沿用当前任务的模型与权限。';
    });
  }
  async function stop() {
    if (!activeTurn) return;
    await action(async () => { await codepetGateway('turn.interrupt', { conversation, turn: activeTurn!.resource }); });
  }
  async function resume() {
    await action(async () => { const generation = epoch; const snapshot = await codepetGateway('conversation.inputQueue.resume', { conversation, expectedRevision: queue!.revision }); if (generation === epoch) queue = snapshot; });
  }
  function restoreDraft() {
    const backups = conversationDraftBackups.get(key) ?? []; const old = backups.pop(); if (!old) return;
    if (text.trim()) backups.push({ text, selection }); text = old.text; selection = old.selection; backupCount = backups.length; textarea?.focus();
  }
  onMount(() => {
    mounted = true;
    let closed = false, unlisten: (() => void) | undefined;
    void listen<ProtocolEvent>('codepet-gateway-event', ({ payload: event }) => {
      if (event.method === 'conversation.inputQueueChanged' && same(event.params.payload.conversation)) void refreshQueue();
      if (event.method === 'turn.upserted' && same(event.params.payload.turn.conversation)) {
        liveRevision++; const turn = event.params.payload.turn;
        if (['completed','failed','interrupted'].includes(turn.status)) { if (!activeTurn || activeTurn.resource.nativeResourceId === turn.resource.nativeResourceId) { activeTurn = null; status = 'idle'; } }
        else { activeTurn = turn; status = turn.status === 'waiting-approval' ? 'waiting-approval' : 'running'; }
      }
      if (event.method === 'conversation.upserted' && same(event.params.payload.conversation.resource)) { liveRevision++; activeTurn = event.params.payload.conversation.activeTurn ?? null; status = event.params.payload.conversation.status; }
      if (event.method === 'provider.changed' && event.params.payload.provider.id === conversation.providerId) { ready = event.params.payload.provider.runtime.status === 'ready'; if (ready) void initialize(); }
    }).then(fn => { if (closed) fn(); else unlisten = fn; }).catch(() => {});
    const timer = setInterval(() => void refreshQueue(), 3000);
    return () => { closed = true; mounted = false; epoch++; clearInterval(timer); unlisten?.(); };
  });
</script>
<div class="conversation-composer" aria-label="会话输入与待发队列">
  {#if supportsQueue}
    <div class="queue-heading"><strong>待发送 {queue?.items.length ?? 0} 条{queue?.paused ? ' · 已暂停' : ''}</strong><div>{#if queue?.paused && queue.items.length}<button disabled={busy || !ready} on:click={resume}>继续队列</button>{/if}<button disabled={loading} on:click={initialize}>刷新</button></div></div>
    <div class="queue-items" aria-label="待发消息">
      {#each queue?.items ?? [] as item (item.id)}
        <article><p class="preview">{item.input.text}</p>
          {#if item.state === 'outcomeUnknown'}<p class="status">发送结果未知，请核对会话。</p>{:else if !editable(item)}<p class="status">{item.state === 'steering' ? '正在引导…' : '正在发送…'}</p>{/if}
          {#if item.errorMessage}<p class="status">{item.errorMessage}</p>{/if}
          <div class="actions"><button disabled={busy || !canSteer || !editable(item)} on:click={() => steer(item)}>引导</button><button disabled={busy || !ready || !editable(item)} on:click={() => withdraw(item, true)}>重新编辑</button><button disabled={busy || !ready || (!editable(item) && item.state !== 'outcomeUnknown')} on:click={() => withdraw(item, false)}>{item.state === 'outcomeUnknown' ? '核对后移除' : '删除'}</button></div>
        </article>
      {/each}
      {#each queue?.recoverableEdits ?? [] as item (item.id)}<article><p class="preview">{item.input.text}</p><button disabled={busy || !ready} on:click={() => action(() => restore(item))}>恢复编辑</button></article>{/each}
    </div>
  {/if}
  <form on:submit|preventDefault={() => send()}>
    <label>继续对话<textarea bind:this={textarea} bind:value={text} rows="3" placeholder={activeTurn ? '输入消息加入队列，或引导当前任务…' : '输入消息…'} disabled={status === 'archived'}></textarea></label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}{#if notice}<p class="status" role="status">{notice}</p>{/if}
    <div class="actions submit-actions">
      {#if backupCount}<button type="button" on:click={restoreDraft}>恢复之前的草稿</button>{/if}
      {#if pending}<button type="button" disabled={busy || !ready} on:click={() => send(true)}>核对上次入队结果</button>{/if}
      {#if activeTurn && capabilities?.methods.includes('turn.interrupt')}<button type="button" disabled={busy || !ready} on:click={stop}>停止任务</button>{/if}
      <button type="submit" disabled={busy || loading || !ready || pending || !text.trim() || status === 'archived' || (supportsQueue && !queue)}>{busy ? '处理中…' : activeTurn || queue?.items.length ? '加入队列' : '发送消息'}</button>
    </div>
  </form>
</div>
<style>
  .conversation-composer{padding:12px 14px;border-top:1px solid var(--color-main-divider);flex:none;min-width:0}.queue-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:12px}.queue-heading>div,.actions{display:flex;flex-wrap:wrap;gap:6px}.queue-items{max-height:180px;overflow:auto;overscroll-behavior:contain}article{padding:8px;margin:6px 0;background:var(--color-content-bg,var(--app-panel));border:1px solid var(--color-main-divider);border-radius:8px}.preview{display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;white-space:pre-wrap;overflow-wrap:anywhere;margin:0 0 6px;font-size:13px}.status,.error{font-size:12px;overflow-wrap:anywhere;margin:6px 0}.status{color:var(--color-text-secondary,var(--app-muted))}.error{color:var(--color-main-danger-text)}label{display:block;font-size:12px}textarea{box-sizing:border-box;display:block;width:100%;resize:vertical;min-height:58px;max-height:160px;margin:6px 0}.submit-actions{justify-content:flex-end}button{font:inherit;font-size:12px;white-space:nowrap;min-height:28px;padding:4px 9px;border:1px solid var(--app-border);border-radius:6px;background:var(--app-surface-soft);color:var(--app-text);cursor:pointer}button:hover:not(:disabled){background:var(--color-main-hover)}button:disabled{opacity:.45;cursor:default}button:focus-visible,textarea:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}button[type=submit]{background:var(--color-main-accent-solid);color:var(--color-main-on-accent);border-color:transparent}form{margin-top:6px}@media(max-width:400px){.conversation-composer{padding:10px}.queue-items{max-height:140px}.queue-heading{flex-wrap:wrap}}
</style>
