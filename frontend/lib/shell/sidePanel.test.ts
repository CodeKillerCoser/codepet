// @vitest-environment jsdom
import { afterEach, expect, test, vi } from 'vitest';
import { tick, type Snippet } from 'svelte';
import { get } from 'svelte/store';
import { createSidePanel, type SidePanelContent } from './sidePanel';

const content = (() => {}) as unknown as Snippet;
const entry = (overrides: Partial<SidePanelContent> = {}): SidePanelContent => ({
  title: 'Details', description: 'Selected item', closeLabel: 'Close details',
  content, onClose: vi.fn(), ...overrides,
});
afterEach(() => document.body.replaceChildren());

test('a page cannot clear a replacement panel owned by another page', () => {
  const panel = createSidePanel();
  const ownerA = Symbol(), ownerB = Symbol();
  const first = entry(), second = entry();
  panel.show(ownerA, first);
  panel.show(ownerB, second);
  panel.release(ownerA);
  expect(first.onClose).toHaveBeenCalledOnce();
  expect(second.onClose).not.toHaveBeenCalled();
  expect(get(panel.current)?.owner).toBe(ownerB);
  panel.release(ownerB);
  expect(get(panel.current)).toBeNull();
});

test('explicit close restores its trigger, while route cleanup preserves navigation focus', async () => {
  const panel = createSidePanel();
  const trigger = document.createElement('button'), navigation = document.createElement('button');
  document.body.append(trigger, navigation);
  const detail = entry({ returnFocus: trigger });
  panel.show(Symbol(), detail);
  navigation.focus();
  panel.dismiss();
  await tick();
  expect(detail.onClose).toHaveBeenCalledOnce();
  expect(document.activeElement).toBe(trigger);
  panel.show(Symbol(), detail);
  navigation.focus();
  panel.dismiss(false);
  await tick();
  expect(document.activeElement).toBe(navigation);
});

test('a new panel cancels an old pending focus restoration', async () => {
  const panel = createSidePanel();
  const oldTrigger = document.createElement('button'), newTrigger = document.createElement('button');
  document.body.append(oldTrigger, newTrigger);
  panel.show(Symbol(), entry({ returnFocus: oldTrigger }));
  panel.dismiss();
  panel.show(Symbol(), entry({ returnFocus: newTrigger }));
  newTrigger.focus();
  await tick();
  expect(document.activeElement).toBe(newTrigger);
});

test('framework state is isolated between app instances', () => {
  const first = createSidePanel(), second = createSidePanel();
  first.show(Symbol(), entry());
  first.setMode('column');
  expect(get(second.current)).toBeNull();
  expect(get(second.mode)).toBe('drawer');
});
