// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import Composer from './ConversationComposer.svelte';
import { conversationDrafts, conversationDraftBackups, pendingQueueSubmissions } from './conversationInputDrafts';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const events = vi.hoisted(() => ({ callback: undefined as ((event: any) => void) | undefined }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (_name, callback) => { events.callback = callback; return () => { events.callback = undefined; }; }) }));
const route = { providerId: 'p', nativeResourceId: 'c' };
const active = { resource: { providerId: 'p', nativeResourceId: 't' }, conversation: route, status: 'running' };
const item = (id: string, text: string) => ({ id, input: { kind: 'text', text }, selection: {}, capabilityRevision: 'r', sequence: 1, state: 'queued', createdAt: 0 });
let queue: any, calls: any[], component: ReturnType<typeof mount>;
let enqueue: ((p: any) => Promise<any>) | undefined;
const settle = async () => { for (let i = 0; i < 10; i++) { await Promise.resolve(); await tick(); } };
const button = (text: string) => [...document.querySelectorAll('button')].find(b => b.textContent === text)!;
const input = () => document.querySelector('textarea')!;
async function type(text: string) { input().value = text; input().dispatchEvent(new Event('input', { bubbles: true })); await settle(); }
beforeEach(async () => {
  queue = { conversation: route, revision: 1, paused: false, items: [item('a', 'queued text')], recoverableEdits: [], snapshotCursor: '1' };
  calls = []; enqueue = undefined;
  conversationDrafts.clear(); conversationDraftBackups.clear(); pendingQueueSubmissions.clear();
  vi.mocked(invoke).mockImplementation(async (_command, args: any) => {
    const { id, method, params } = args.request; calls.push({ method, params });
    let result: any;
    if (method === 'provider.describe') result = { provider: { runtime: { status: 'ready' } }, capabilities: { revision: 'r', methods: ['conversation.inputQueue.enqueue','conversation.inputQueue.get','conversation.inputQueue.steer','turn.interrupt'] } };
    else if (method === 'conversation.get') result = { conversation: { resource: route, status: 'running', activeTurn: active } };
    else if (method === 'conversation.acquireInteraction') result = { selection: { accessModeId: 'workspace-write' } };
    else if (method === 'conversation.inputQueue.get') result = structuredClone(queue);
    else if (method === 'conversation.inputQueue.withdraw') {
      const removed = queue.items.find((i: any) => i.id === params.itemId); queue.items = queue.items.filter((i: any) => i !== removed); queue.revision++;
      result = { item: { ...removed, state: 'withdrawn' }, revision: queue.revision };
    } else if (method === 'conversation.inputQueue.enqueue') result = enqueue ? await enqueue(params) : { item: item('b', params.input.text), revision: 2 };
    else if (method === 'conversation.inputQueue.steer') { result = { item: { ...queue.items[0], state: 'accepted' }, revision: ++queue.revision }; queue.items = []; }
    else result = queue;
    return { jsonrpc: '2.0', id, result };
  });
  component = mount(Composer, { target: document.body, props: { conversation: route } });
  await settle();
});
afterEach(async () => { await unmount(component); document.body.innerHTML = ''; });
it('withdraws before editing, focuses the composer, and preserves its previous draft', async () => {
  await type('existing draft'); button('重新编辑').click(); await settle();
  expect(input().value).toBe('queued text'); expect(document.activeElement).toBe(input());
  expect(input().selectionStart).toBe('queued text'.length);
  expect(calls.filter(c => c.method.includes('withdraw'))[0].params).toMatchObject({ expectedRevision: 1, kind: 'edit' });
  expect(calls.some(c => c.method.endsWith('acknowledgeEdit'))).toBe(true);
  button('恢复之前的草稿').click(); await settle(); expect(input().value).toBe('existing draft');
});
it('steers into the active turn without starting another turn', async () => {
  button('引导').click(); await settle();
  expect(calls.find(c => c.method.endsWith('.steer')).params.turn).toEqual(active.resource);
  expect(calls.some(c => c.method === 'turn.send')).toBe(false);
  expect(document.body.textContent).toContain('引导已受理');
});
it('keeps newly typed text when an older enqueue acknowledgement arrives', async () => {
  let resolve!: (value: any) => void;
  enqueue = () => new Promise(r => { resolve = r; });
  await type('first input'); button('加入队列').click(); await settle();
  await type('new draft'); resolve({ item: item('b', 'first input'), revision: 2 }); await settle();
  expect(input().value).toBe('new draft'); expect(pendingQueueSubmissions.size).toBe(0);
});
it('reuses the original request after an acknowledgement is lost and deletion preserves the draft', async () => {
  enqueue = async () => { throw new Error('connection lost'); };
  await type('draft'); button('加入队列').click(); await settle();
  const original = calls.find(c => c.method.endsWith('.enqueue')).params;
  expect(original.selection).toEqual({ accessModeId: 'workspace-write' });
  enqueue = undefined; button('核对上次入队结果').click(); await settle();
  expect(calls.filter(c => c.method.endsWith('.enqueue'))[1].params).toEqual(original);
  await type('keep me'); button('删除').click(); await settle(); expect(input().value).toBe('keep me');
});

it('consumes canonical event payloads, refreshes queue, and disables steering after the turn ends', async () => {
  queue.items.push(item('b', 'from phone')); queue.revision++;
  events.callback!({ payload: { jsonrpc: '2.0', method: 'conversation.inputQueueChanged', params: { eventCursor: '2', payload: { conversation: route, revision: 2 } } } });
  await settle(); expect(document.body.textContent).toContain('from phone');
  events.callback!({ payload: { jsonrpc: '2.0', method: 'turn.upserted', params: { eventCursor: '3', payload: { turn: { ...active, status: 'completed' } } } } });
  await settle(); expect(button('引导').disabled).toBe(true);
});
it('sends a new message without exposing or invoking a separate resume control', async () => {
  queue.paused = true;
  await type('继续执行');
  expect([...document.querySelectorAll('button')].some(b => b.textContent === '继续队列')).toBe(false);
  button('加入队列').click(); await settle();
  expect(calls.some(c => c.method === 'conversation.inputQueue.enqueue' && c.params.input.text === '继续执行')).toBe(true);
  expect(calls.some(c => c.method === 'conversation.inputQueue.resume')).toBe(false);
});
