import { gradientCss, gradientEditorFromCss, type GradientEditorValue } from "./gradientColor";
import type { AppSettings } from "./types";

export type RunningBubbleColorKey = "backgroundColor" | "borderColor";

export function updateRunningBubbleColorSetting(
  settings: AppSettings,
  key: RunningBubbleColorKey,
  fallback: string,
  patch: Partial<GradientEditorValue>,
): AppSettings {
  const current = gradientEditorFromCss(settings.appearance.runningBubble[key], fallback);
  const nextColor = gradientCss({ ...current, ...patch });

  return {
    ...settings,
    appearance: {
      ...settings.appearance,
      runningBubble: {
        ...settings.appearance.runningBubble,
        [key]: nextColor,
      },
    },
  };
}

export function colorStopIndexFromBand(count: number, offsetX: number, width: number) {
  const safeCount = Math.max(1, Math.floor(count || 1));
  const safeWidth = Math.max(1, width || 1);
  const ratio = Math.min(0.999999, Math.max(0, offsetX / safeWidth));
  return Math.min(safeCount - 1, Math.floor(ratio * safeCount));
}
