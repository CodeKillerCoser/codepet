import { derived, writable } from "svelte/store";

export type AppRoute = "tasks" | "worktrees" | "knowledge" | "connections" | "devices" | "runtimes" | "usage" | "personalize" | "events" | "settings" | "appearance" | "notifications" | "permissions" | "models" | "extraction" | "about";
export const connectionRoutes = [
  { route: "connections", label: "活动接入" },
  { route: "runtimes", label: "本机运行时" },
] as const;
export const settingsRoutes = [
  { route: "settings", label: "通用" },
  { route: "appearance", label: "外观" },
  { route: "personalize", label: "桌宠" },
  { route: "notifications", label: "通知" },
  { route: "models", label: "模型配置" },
  { route: "permissions", label: "权限" },
  { route: "events", label: "诊断" },
  { route: "about", label: "关于" },
] as const;
export const isSettingsRoute = (route: AppRoute) => settingsRoutes.some(entry => entry.route === route);
export const isConnectionRoute = (route: AppRoute) => connectionRoutes.some(entry => entry.route === route);
export const routeTitle = (route: AppRoute) => settingsRoutes.find(entry => entry.route === route)?.label
  ?? (isConnectionRoute(route)
    ? "智能体"
    : route === "tasks"
      ? "工作台"
      : (route === "worktrees" || route === "extraction")
        ? "工作树"
        : route === "knowledge"
          ? "工作记忆"
          : route === "devices"
            ? "远程控制"
            : "用量");

export function createNavigation(initial: AppRoute = "tasks") {
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
