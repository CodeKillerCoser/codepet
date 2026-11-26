import { derived, get, writable } from 'svelte/store';

export const SIDEBAR_DEFAULT_WIDTH = 200;
export const SIDEBAR_MIN_WIDTH = 164;
export const SIDEBAR_MAX_WIDTH = 280;
export const SIDEBAR_MAX_VIEWPORT_RATIO = 0.34;
export const SIDEBAR_WIDTH_STORAGE_KEY = 'codepet.sidebar.width';

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

export function sidebarMaxWidth(viewportWidth = 0): number {
  if (!Number.isFinite(viewportWidth) || viewportWidth <= 0) return SIDEBAR_MAX_WIDTH;
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, Math.floor(viewportWidth * SIDEBAR_MAX_VIEWPORT_RATIO)));
}

export function clampSidebarWidth(next: number, viewportWidth = 0): number {
  if (!Number.isFinite(next)) return SIDEBAR_DEFAULT_WIDTH;
  const max = sidebarMaxWidth(viewportWidth);
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(max, Math.round(next)));
}

export function readPersistedSidebarWidth(storage?: StorageLike | null): number {
  try {
    const target = storage === undefined ? globalThis.localStorage : storage;
    const raw = target?.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
    if (raw == null || raw === '') return SIDEBAR_DEFAULT_WIDTH;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? clampSidebarWidth(parsed) : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

export function writePersistedSidebarWidth(width: number, storage?: StorageLike | null): void {
  try {
    const target = storage === undefined ? globalThis.localStorage : storage;
    target?.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(clampSidebarWidth(width)));
  } catch {
    // Ignore storage quota / privacy mode failures.
  }
}

export function createSidebarWidth(storage?: StorageLike | null) {
  const viewportWidth = writable(0);
  const preferredWidth = writable(readPersistedSidebarWidth(storage));
  const resizing = writable(false);
  const bounds = derived(viewportWidth, (viewport) => ({
    min: SIDEBAR_MIN_WIDTH,
    max: sidebarMaxWidth(viewport),
  }));
  const width = derived([preferredWidth, viewportWidth], ([preferred, viewport]) =>
    clampSidebarWidth(preferred, viewport),
  );

  function setWidth(next: number, persist = true) {
    if (!Number.isFinite(next)) return;
    const clamped = clampSidebarWidth(next, get(viewportWidth));
    preferredWidth.set(clamped);
    if (persist) writePersistedSidebarWidth(clamped, storage);
  }

  return {
    width: { subscribe: width.subscribe },
    bounds: { subscribe: bounds.subscribe },
    resizing: { subscribe: resizing.subscribe },
    setViewportWidth(next: number) {
      if (Number.isFinite(next) && next > 0) viewportWidth.set(next);
    },
    setWidth,
    resetWidth() {
      setWidth(SIDEBAR_DEFAULT_WIDTH, true);
    },
    setResizing: resizing.set,
  };
}

export type SidebarWidthController = ReturnType<typeof createSidebarWidth>;
