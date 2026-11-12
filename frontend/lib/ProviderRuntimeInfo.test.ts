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
