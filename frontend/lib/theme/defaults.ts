import type { AppSettings, PixelPetSprite, TaskStatus } from "../types";

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

export const defaultPetSprite: PixelPetSprite = {
  body: "#f4c04e",
  accent: "#1f2937",
  eyes: "#2563eb",
};

export const gradientStopPalette = ["#0066ee", "#eecc00", "#c946ff", "#00d084", "#ff7a00", "#00c2ff"] as const;

export const cssColorTokens = {
  transparent: "var(--color-transparent)",
  white: "var(--color-white)",
  petSpriteSoftWhite: "var(--asset-pet-sprite-soft-white)",
  petSpriteWhite: "var(--asset-pet-sprite-white)",
  petSpriteFoot: "var(--asset-pet-sprite-foot)",
} as const;

export function themeClassNames(mode: "light" | "dark"): string {
  return mode === "dark" ? "theme-dark dark dark-theme" : "theme-light light light-theme";
}

export function spriteAccentColorForStatus(value: TaskStatus): string {
  if (value === "waiting-approval") return "var(--asset-pet-sprite-waiting)";
  if (value === "failed") return "var(--asset-pet-sprite-failed)";
  if (value === "done") return "var(--asset-pet-sprite-done)";
  if (value === "running") return "var(--asset-pet-sprite-running)";
  if (value === "thinking") return "var(--asset-pet-sprite-thinking)";
  return "var(--asset-pet-sprite-idle)";
}
