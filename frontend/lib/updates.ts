import type { AppSettings, AppUpdate } from "./types";

export type UpdateCheckMode = "manual" | "auto";

export function shouldPromptForUpdate(
  update: AppUpdate | null,
  mode: UpdateCheckMode,
  settings: Pick<AppSettings, "updates"> | null | undefined,
): update is AppUpdate {
  if (!update) {
    return false;
  }
  if (mode === "manual") {
    return true;
  }
  return settings?.updates?.ignoredVersion !== update.version;
}

export function ignoredUpdateSettings(version: string): AppSettings["updates"] {
  return {
    ignoredVersion: version,
  };
}
