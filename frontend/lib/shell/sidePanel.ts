import { getContext, setContext, tick, type Snippet } from 'svelte';
import { get, writable } from 'svelte/store';

const contextKey = Symbol('app-side-panel');
export type SidePanelMode = 'column' | 'drawer';
export interface SidePanelContent {
  title: string;
  description: string;
  closeLabel: string;
  content: Snippet;
  returnFocus?: HTMLElement;
  onClose: () => void;
}
export interface SidePanelEntry extends SidePanelContent { owner: symbol }

// One controller per App instance. Feature pages never own the host or its geometry.
export function createSidePanel() {
  const current = writable<SidePanelEntry | null>(null);
  const mode = writable<SidePanelMode>('drawer');
  let revision = 0;
  return {
    current: { subscribe: current.subscribe },
    mode: { subscribe: mode.subscribe },
    setMode: mode.set,
    show(owner: symbol, content: SidePanelContent) {
      const previous = get(current);
      ++revision;
      current.set({ ...content, owner });
      if (previous && previous.owner !== owner) previous.onClose();
    },
    release(owner: symbol) {
      if (get(current)?.owner !== owner) return;
      ++revision;
      current.set(null);
    },
    dismiss(restoreFocus = true) {
      const entry = get(current);
      if (!entry) return;
      const closedRevision = ++revision;
      current.set(null);
      entry.onClose();
      if (restoreFocus) void tick().then(() => {
        if (revision === closedRevision && entry.returnFocus?.isConnected) {
          entry.returnFocus.focus({ preventScroll: true });
        }
      });
    },
  };
}
export type SidePanelController = ReturnType<typeof createSidePanel>;
export function provideSidePanel() {
  const controller = createSidePanel();
  setContext(contextKey, controller);
  return controller;
}
export function useSidePanel(): SidePanelController {
  const controller = getContext<SidePanelController | undefined>(contextKey);
  if (!controller) throw new Error('Side panel content requires an App framework host');
  return controller;
}
