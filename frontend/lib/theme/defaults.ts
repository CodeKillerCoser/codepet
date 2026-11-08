import type { AppSettings } from "../types";

export const defaultRunningBubbleColors = {
  background: "#e8f2ff",
  border: "#3d73d8",
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

export function themeClassNames(mode: "light" | "dark"): string {
  return mode === "dark" ? "theme-dark dark dark-theme" : "theme-light light light-theme";
}
