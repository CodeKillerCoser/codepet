// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import ProviderRuntimeInfo from './ProviderRuntimeInfo.svelte';
let component: ReturnType<typeof mount>;
afterEach(async () => { await unmount(component); document.body.innerHTML = ''; });
const runtime = { providerId: 'custom', displayName: 'Custom Provider', status: 'not-required' as const, resolvedExecutable: null, source: null, configuredExecutable: null, version: null, diagnostic: null, installed: [] };
it('offers removal only when the parent allows it and disables it during an operation', async () => {
  const onRemove = vi.fn();
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime, connection: undefined, onSelect: vi.fn(), onRemove } });
  await tick();
  const button = document.querySelector<HTMLButtonElement>('button[aria-label="移除 Custom Provider"]')!;
  expect(button).not.toBeNull(); button.click(); expect(onRemove).toHaveBeenCalledWith(runtime);
});
it('keeps built-in providers free of a removal action', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime, connection: undefined, onSelect: vi.fn() } });
  await tick(); expect(document.querySelector('button')).toBeNull();
});
it('disables removal while busy', async () => {
  const onRemove = vi.fn();
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime, connection: undefined, onSelect: vi.fn(), onRemove, busy: true } });
  await tick(); const button = document.querySelector<HTMLButtonElement>('button')!;
  expect(button.disabled).toBe(true); button.click(); expect(onRemove).not.toHaveBeenCalled();
});
it('hides runtime controls for notification observers and self-managed agents', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime: { ...runtime, status: 'ready' }, connection: undefined, onSelect: vi.fn(), kind: 'observation' } });
  await tick();
  expect(document.body.textContent).toContain('通知助手');
  expect(document.querySelector('select')).toBeNull();
  expect(document.body.textContent).not.toContain('由 Provider 管理');
});

it('shows composed features for a zero-instance plugin without querying Gateway methods', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime,
    connection: { pluginId: 'custom', providerId: 'custom', features: ['agent', 'observation'], connectionStatus: 'online', generation: 1, instances: [] },
    onSelect: vi.fn() } });
  await tick();
  expect(document.body.textContent).toContain('智能体');
  expect(document.body.textContent).toContain('通知助手');
  expect(document.querySelector('select')).toBeNull();
});
it('offers runtime selection only for an explicit runtime inventory feature', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime: { ...runtime, status: 'ready' },
    connection: { providerId: 'custom', features: ['agent', 'runtimeInventory'], connectionStatus: 'online', generation: 1, instances: [] },
    onSelect: vi.fn() } });
  await tick(); expect(document.querySelector('select')).not.toBeNull();
});

it('requires an explicit inventory feature instead of guessing from runtime data', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: {
    runtime: { ...runtime, status: 'ready', installed: [{ executablePath: '/legacy/runtime', version: '1', source: 'current-path' }] },
    connection: { providerId: 'legacy', connectionStatus: 'online', generation: 1, instances: [] }, onSelect: vi.fn() } });
  await tick(); expect(document.querySelectorAll('.kind')).toHaveLength(0);
  expect(document.querySelector('select')).toBeNull();
});

it('keeps all four detail rows for self-managed agents and places icon-only removal after details', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime,
    connection: { providerId: 'custom', displayName: 'Long custom agent name', version: '1.2.3', features: ['agent'], connectionStatus: 'online', generation: 1, instances: [] },
    onSelect: vi.fn(), onRemove: vi.fn() } });
  await tick();
  expect([...document.querySelectorAll('dt')].map(node => node.textContent)).toEqual(['插件版本', '运行时路径', '运行时版本', '默认工作目录']);
  expect([...document.querySelectorAll('dd')].map(node => node.textContent?.trim())).toEqual(['1.2.3', '未提供', '未提供', '未提供']);
  const remove = document.querySelector('button')!;
  expect(remove.textContent?.trim()).toBe('');
  expect(document.querySelector('header button')).toBeNull();
  expect(document.querySelector('.provider-details')?.contains(remove)).toBe(true);
  expect(document.querySelector('h3')?.title).toBe('Long custom agent name');
  expect(document.querySelector('label')).toBeNull();
});
it('retains the common detail layout while marking observer runtimes as unnecessary', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime, kind: 'observation', connection: undefined, onSelect: vi.fn() } });
  await tick();
  expect(document.querySelectorAll('dt')).toHaveLength(4);
  expect([...document.querySelectorAll('dd')].map(node => node.textContent?.trim())).toEqual(['未提供', '不需要', '不需要', '未提供']);
});

it('does not invent a category for a plugin without declared features', async () => {
  component = mount(ProviderRuntimeInfo, { target: document.body, props: { runtime, connection: undefined, onSelect: vi.fn() } });
  await tick(); expect(document.querySelectorAll('.kind')).toHaveLength(0);
});
