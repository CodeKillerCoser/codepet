import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";
import { version } from "../../package.json";
declare const __WEB_BUILD_AT__: number;

export const frontendBuild = { version, builtAt: __WEB_BUILD_AT__ };
export interface WebcontentInfo { version: string; builtAt: number; canReload: boolean }
export const extractionDraftDirty = writable(false);
export const webcontentReloading = writable(false);
export const webcontentInfo = () => invoke<WebcontentInfo>("webcontent_info");
export const loadLatestWebcontent = () => invoke<WebcontentInfo>("load_latest_webcontent");

export function needsPageReload(current: Pick<WebcontentInfo, "version" | "builtAt">, next: WebcontentInfo) {
  return current.version !== next.version || current.builtAt !== next.builtAt;
}
const reloadKey = "codepet.webcontent-reload";
export function prepareReload() {
  sessionStorage.setItem(reloadKey, "pending");
}
export function consumeReloadNotice() {
  try {
    const notice = sessionStorage.getItem(reloadKey) === "pending";
    sessionStorage.removeItem(reloadKey);
    return notice;
  } catch { return false; }
}
export function reloadPage(info: WebcontentInfo) {
  const url = new URL(window.location.href);
  url.searchParams.set("webcontent", String(info.builtAt));
  window.location.replace(url.href);
}
