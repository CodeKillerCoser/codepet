import type { AppSettings, ThemePresetId } from "../types";

export const defaultThemePreset: ThemePresetId = "warm-stone";

export interface ThemePresetDefinition {
  id: ThemePresetId;
  name: string;
  description: string;
}

export const themePresets: readonly ThemePresetDefinition[] = [
  { id: "warm-stone", name: "暖石琥珀", description: "暖石中性底与克制琥珀强调" },
  { id: "cool-slate", name: "冷灰蓝墨", description: "冷静岩灰与清晰墨蓝强调" },
  { id: "forest-sage", name: "森林鼠尾草", description: "柔和鼠尾草灰与温润森绿" },
  { id: "caramel-earth", name: "焦糖暖棕", description: "温润纸白与浓郁焦糖大地" },
] as const;

const validThemePresets = new Set<string>(themePresets.map((preset) => preset.id));

export function normalizeThemePreset(value: string | null | undefined): ThemePresetId {
  if (value && validThemePresets.has(value)) {
    return value as ThemePresetId;
  }
  return defaultThemePreset;
}

export const defaultRunningBubbleColors = {
  background: "#fef3dc",
  border: "#d9822b",
} as const;

export const defaultColorFallback = "#000000";

export const defaultRunningBubbleSettings: AppSettings["appearance"]["runningBubble"] = {
  backgroundBreathing: true,
  borderMarquee: false,
  backgroundColor: defaultRunningBubbleColors.background,
  borderColor: defaultRunningBubbleColors.border,
  borderWidth: 1,
  animationMs: 1800,
};

export const gradientStopPalette = ["#0066ee", "#eecc00", "#c946ff", "#00d084", "#ff7a00", "#00c2ff"] as const;

export const cssColorTokens = {
  transparent: "var(--color-transparent)",
  white: "var(--color-white)",
} as const;

export function themeClassNames(mode: "light" | "dark", preset: ThemePresetId | null | undefined = defaultThemePreset): string {
  const modeClasses = mode === "dark" ? "theme-dark dark dark-theme" : "theme-light light light-theme";
  return `${modeClasses} preset-${normalizeThemePreset(preset)}`;
}
