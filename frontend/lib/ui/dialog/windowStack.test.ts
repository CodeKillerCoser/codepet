// @vitest-environment jsdom
import { expect, test } from 'vitest';
import { get } from 'svelte/store';
import { createWindowStack, windowHost } from './windowStack';

test('new windows cover closing windows and out-of-order cleanup preserves the top window', () => {
  const stack = createWindowStack();
  const drawer = stack.open(), preview = stack.open();
  expect(preview.layer).toBeGreaterThan(drawer.layer + 1);
  expect(get(stack.top)).toBe(preview.id);
  drawer.close();
  expect(get(stack.top)).toBe(preview.id);
  preview.close();
  const next = stack.open();
  expect(next.layer).toBeGreaterThan(preview.layer + 1);
  preview.close();
  expect(get(stack.top)).toBe(next.id);
  next.close();
  expect(get(stack.top)).toBeUndefined();
});

test('closing a nested modal restores the previous top window', () => {
  const stack = createWindowStack();
  const drawer = stack.open(), modal = stack.open();
  modal.close();
  expect(get(stack.top)).toBe(drawer.id);
});

test('all windows in a theme share a host outside page stacking contexts', () => {
  const theme = document.createElement('main');
  theme.className = 'main-theme';
  const page = document.createElement('section'), anchor = document.createElement('span');
  theme.append(page); page.append(anchor); document.body.append(theme);
  const host = windowHost(anchor);
  expect(host.parentElement).toBe(theme);
  expect(windowHost(page)).toBe(host);
  expect(page.contains(host)).toBe(false);
  theme.remove();
});
