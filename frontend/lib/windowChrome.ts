import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type WindowPlatform = "macos" | "windows" | "other";

// Keep platform detection and native window calls out of product pages.
export function windowPlatform(userAgent: string): WindowPlatform {
  if (/Macintosh|Mac OS X/.test(userAgent)) return "macos";
  if (/Windows/.test(userAgent)) return "windows";
  return "other";
}

export function createWindowChrome() {
  const platform = windowPlatform(navigator.userAgent);
  const native = isTauri();
  const current = native ? getCurrentWindow() : null;
  return {
    platform,
    native,
    async initialize(onMaximized: (value: boolean) => void) {
      if (!current) return () => {};
      // Install the controls before removing the Windows caption. macOS keeps
      // its native traffic lights in the configured overlay title bar.
      if (platform === "windows") await current.setDecorations(false);
      const sync = async () => onMaximized(await current.isMaximized());
      await sync();
      return current.onResized(() => { void sync(); });
    },
    minimize: async () => { await current?.minimize(); },
    toggleMaximize: async () => { await current?.toggleMaximize(); },
    close: async () => { await current?.close(); },
  };
}
