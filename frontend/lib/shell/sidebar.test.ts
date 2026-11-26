import { get } from 'svelte/store';
import { describe, expect, test } from 'vitest';
import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_WIDTH_STORAGE_KEY,
  clampSidebarWidth,
  createSidebarWidth,
  sidebarMaxWidth,
} from './sidebar';

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => {
      map.set(key, value);
    },
  };
}

describe('sidebar width controller', () => {
  test('clamps widths between fixed min and max bounds', () => {
    expect(clampSidebarWidth(80)).toBe(SIDEBAR_MIN_WIDTH);
    expect(clampSidebarWidth(480)).toBe(SIDEBAR_MAX_WIDTH);
    expect(clampSidebarWidth(224.6)).toBe(225);
    expect(clampSidebarWidth(Number.NaN)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  test('caps maximum width on narrow viewports without dropping below min width', () => {
    expect(sidebarMaxWidth(1280)).toBe(SIDEBAR_MAX_WIDTH);
    expect(sidebarMaxWidth(640)).toBe(217);
    expect(sidebarMaxWidth(420)).toBe(SIDEBAR_MIN_WIDTH);
  });

  test('restores persisted width, clamps updates, and resets to default', () => {
    const storage = memoryStorage({ [SIDEBAR_WIDTH_STORAGE_KEY]: '244' });
    const sidebar = createSidebarWidth(storage);
    sidebar.setViewportWidth(1280);
    expect(get(sidebar.width)).toBe(244);

    sidebar.setWidth(120);
    expect(get(sidebar.width)).toBe(SIDEBAR_MIN_WIDTH);
    expect(storage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(SIDEBAR_MIN_WIDTH));

    sidebar.setWidth(999);
    expect(get(sidebar.width)).toBe(SIDEBAR_MAX_WIDTH);
    expect(storage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(SIDEBAR_MAX_WIDTH));

    sidebar.resetWidth();
    expect(get(sidebar.width)).toBe(SIDEBAR_DEFAULT_WIDTH);
    expect(storage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(SIDEBAR_DEFAULT_WIDTH));
  });
});
