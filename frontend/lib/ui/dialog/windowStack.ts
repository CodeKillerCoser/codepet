import { writable } from 'svelte/store';

export const windowLayerKey = Symbol('global-window-layer');
export const windowTriggerKey = Symbol('global-window-trigger');
let openingTrigger: HTMLElement | undefined;
export function windowOpeningTrigger() {
  return openingTrigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : undefined);
}

// Every shared modal (including drawers) joins the same stack in this WebView.
// Keep increasing the layer so a closing animation cannot cover a new window.
export function createWindowStack() {
  let nextLayer = 1000;
  const active = new Map<symbol, number>();
  const top = writable<symbol | undefined>();
  return {
    top: { subscribe: top.subscribe },
    open() {
      const id = Symbol('window');
      const layer = nextLayer;
      nextLayer += 2;
      active.set(id, layer);
      top.set(id);
      return { id, layer, close() {
        active.delete(id);
        top.set([...active.keys()].at(-1));
      } };
    },
  };
}

export const windowStack = createWindowStack();
const hosts = new WeakMap<Element, HTMLElement>();
export function windowHost(anchor: Element) {
  const theme = anchor.closest('.main-theme') ?? anchor.ownerDocument.body;
  let host = hosts.get(theme);
  if (!host) {
    host = anchor.ownerDocument.createElement('div');
    host.className = 'app-window-host';
    theme.append(host);
    hosts.set(theme, host);
    theme.addEventListener('click', event => {
      // WebKit does not focus pointer-clicked buttons. Remember the actual
      // opener so a modal returns to it even when activeElement stayed behind.
      const target = (event.target as Element).closest<HTMLElement>('button, a[href], input, select, textarea, [tabindex]');
      openingTrigger = target ?? undefined;
      setTimeout(() => { if (openingTrigger === target) openingTrigger = undefined; }, 0);
    }, { capture: true });
  }
  return host;
}
