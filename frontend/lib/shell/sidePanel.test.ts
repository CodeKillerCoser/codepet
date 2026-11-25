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

test('column width stays within usable bounds and survives content changes', () => {
  const panel = createSidePanel();
  panel.setViewportWidth(1200);
  expect(get(panel.bounds)).toEqual({ min: 400, max: 900 });
  panel.setWidth(1000);
  expect(get(panel.width)).toBe(900);
  panel.setWidth(100);
  expect(get(panel.width)).toBe(400);
  panel.setWidth(480);
  panel.show(Symbol(), entry());
  panel.dismiss(false);
  panel.show(Symbol(), entry());
  expect(get(panel.width)).toBe(480);
  panel.setWidth(NaN);
  expect(get(panel.width)).toBe(480);
  expect(get(createSidePanel().width)).toBe(400);
});

test('window resizing re-evaluates both proportional boundaries and restores the chosen column width', () => {
  const panel = createSidePanel();
  panel.setViewportWidth(1440);
  panel.setWidth(1000);
  panel.setViewportWidth(1180);
  expect(get(panel.bounds)).toEqual({ min: 1180 / 3, max: 885 });
  expect(get(panel.width)).toBe(885);
  panel.setViewportWidth(1440);
  expect(get(panel.width)).toBe(1000);
  panel.setWidth(500);
  panel.setViewportWidth(1800);
  expect(get(panel.width)).toBe(600);
  panel.setViewportWidth(1440);
  expect(get(panel.width)).toBe(500);
  panel.resetWidth();
  expect(get(panel.width)).toBe(480);
  panel.setViewportWidth(1200);
  expect(get(panel.width)).toBe(400);
  panel.setViewportWidth(Infinity);
  expect(get(panel.bounds).max).toBe(900);
});

test('drawer width clamps to one third through three quarters of the viewport', () => {
  const panel = createSidePanel();
  panel.setViewportWidth(900);
  expect(get(panel.drawerWidth)).toBe(450);
  expect(get(panel.drawerBounds)).toEqual({ min: 300, max: 675 });
  panel.setDrawerWidth(100);
  expect(get(panel.drawerWidth)).toBe(300);
  panel.setDrawerWidth(1000);
  expect(get(panel.drawerWidth)).toBe(675);
  panel.setDrawerWidth(NaN);
  panel.setViewportWidth(0);
  expect(get(panel.drawerWidth)).toBe(675);
});

test('drawer ratio survives viewport and mode changes independently of the column', () => {
  const panel = createSidePanel();
  panel.setViewportWidth(1440);
  panel.setWidth(480);
  panel.setViewportWidth(900);
  panel.setDrawerWidth(450);
  panel.setViewportWidth(390);
  expect(get(panel.drawerWidth)).toBe(195);
  panel.setViewportWidth(1440);
  panel.setMode('column');
  expect(get(panel.width)).toBe(480);
  panel.setWidth(600);
  panel.setMode('drawer');
  expect(get(panel.drawerRatio)).toBe(0.5);
  panel.setViewportWidth(900);
  expect(get(panel.drawerWidth)).toBe(450);
  expect(get(createSidePanel().drawerRatio)).toBe(0.5);
});
