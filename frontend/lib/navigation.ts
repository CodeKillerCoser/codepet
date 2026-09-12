import { derived, writable } from "svelte/store";

export type AppRoute = "tasks" | "agents" | "connections" | "usage" | "personalize" | "events" | "settings";
export function createNavigation(initial: AppRoute = "agents") {
  const state = writable<{ entries: AppRoute[]; index: number }>({ entries: [initial], index: 0 });
  return {
    subscribe: derived(state, value => ({ ...value, current: value.entries[value.index], canGoBack: value.index > 0, canGoForward: value.index < value.entries.length - 1 })).subscribe,
    navigate(route: AppRoute) {
      state.update(value => {
        if (value.entries[value.index] === route) return value;
        const entries = [...value.entries.slice(0, value.index + 1), route].slice(-100);
        return { entries, index: entries.length - 1 };
      });
    },
    back: () => state.update(value => ({ ...value, index: Math.max(0, value.index - 1) })),
    forward: () => state.update(value => ({ ...value, index: Math.min(value.entries.length - 1, value.index + 1) })),
  };
}
