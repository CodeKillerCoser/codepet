import type { AppSettings } from "./types";
import { cssColorTokens, defaultColorFallback, defaultRunningBubbleColors, gradientStopPalette } from "./theme";

export type GradientEditorValue = {
  angle: number;
  colors: string[];
};

const hexColorPattern = /^#[0-9a-f]{6}$/i;
const linearGradientPattern = /^linear-gradient\(\s*(\d{1,3})deg\s*,\s*(.+)\)$/i;

export function gradientCss(value: GradientEditorValue): string {
  const colors = safeColorStops(value.colors, defaultColorFallback);
  const angle = clampGradientAngle(value.angle);
  return `linear-gradient(${angle}deg, ${gradientStops(colors)})`;
}

export function gradientSegmentCss(colors: string[]): string {
  const safeColors = safeColorStops(colors, defaultColorFallback);
  const width = 100 / safeColors.length;
  const stops = safeColors.flatMap((color, index) => {
    const start = Math.round(index * width * 100) / 100;
    const end = Math.round((index + 1) * width * 100) / 100;
    return [`${color} ${start}%`, `${color} ${end}%`];
  });
  return `linear-gradient(90deg, ${stops.join(", ")})`;
}

export function gradientEditorFromCss(value: string | null | undefined, fallback: string): GradientEditorValue {
  const fallbackColor = safeHexColor(fallback, defaultColorFallback);
  const current = value?.trim() || fallbackColor;
  const gradient = parseLinearGradient(current);
  if (gradient) return gradient;

  const solid = safeHexColor(current, fallbackColor);
  return {
    angle: 90,
    colors: [solid],
  };
}

export function runningBubbleStyle(runningBubble: AppSettings["appearance"]["runningBubble"]): string {
  const background = colorParts(runningBubble.backgroundColor, defaultRunningBubbleColors.background);
  const border = colorParts(runningBubble.borderColor, defaultRunningBubbleColors.border);
  const duration = Math.min(4000, Math.max(600, Math.round(runningBubble.animationMs || 1800)));
  const borderWidth = Math.min(8, Math.max(1, Math.round(runningBubble.borderWidth || 1)));
  const backgroundBorderColor = border.start;

  return [
    `--pet-running-bubble-bg: ${background.css}`,
    `--pet-running-bubble-bg-layer: ${backgroundLayerValue(background.css)}`,
    `--pet-running-bubble-bg-dim: ${mixedColorValue(background, `88%, ${backgroundBorderColor}`)}`,
    `--pet-running-bubble-bg-dim-layer: ${mixedLayerValue(background, `88%, ${backgroundBorderColor}`)}`,
    `--pet-running-bubble-bg-peak: ${mixedColorValue(background, `76%, ${cssColorTokens.white}`)}`,
    `--pet-running-bubble-bg-peak-layer: ${mixedLayerValue(background, `76%, ${cssColorTokens.white}`)}`,
    `--pet-running-bubble-border: ${border.start}`,
    `--pet-running-bubble-border-cool: ${border.palette[1]}`,
    `--pet-running-bubble-border-light: ${border.palette[2]}`,
    `--pet-running-bubble-border-hot: ${border.palette[3]}`,
    `--pet-running-bubble-border-warm: ${border.palette[4]}`,
    `--pet-running-bubble-border-width: ${borderWidth}px`,
    `--pet-running-bubble-duration: ${duration}ms`,
  ].join("; ");
}

export function nextGradientStopColor(colors: string[]): string {
  const normalized = new Set(colors.map((color) => safeHexColor(color, "").toLowerCase()).filter(Boolean));
  return gradientStopPalette.find((color) => !normalized.has(color)) ?? gradientStopPalette[colors.length % gradientStopPalette.length];
}

function colorParts(value: string, fallback: string) {
  const gradient = parseLinearGradient(value);
  if (gradient) {
    const colors = safeColorStops(gradient.colors, fallback);
    return {
      mode: "linear" as const,
      angle: gradient.angle,
      css: gradientCss({ angle: gradient.angle, colors }),
      colors,
      start: colors[0],
      palette: marqueePalette(colors),
    };
  }

  const color = safeHexColor(value, fallback);
  return {
    mode: "solid" as const,
    angle: 90,
    css: color,
    colors: [color],
    start: color,
    palette: marqueePalette([color]),
  };
}

function mixedColorValue(parts: ReturnType<typeof colorParts>, mix: string): string {
  if (parts.mode === "solid") {
    return `color-mix(in srgb, ${parts.start} ${mix})`;
  }

  return `linear-gradient(${parts.angle}deg, ${gradientStops(parts.colors.map((color) => `color-mix(in srgb, ${color} ${mix})`))})`;
}

function mixedLayerValue(parts: ReturnType<typeof colorParts>, mix: string): string {
  return backgroundLayerValue(mixedColorValue(parts, mix));
}

function backgroundLayerValue(value: string): string {
  return value.startsWith("linear-gradient(") ? value : `linear-gradient(${value} 0 0)`;
}

function parseLinearGradient(value: string | null | undefined): GradientEditorValue | null {
  const match = value?.trim().match(linearGradientPattern);
  if (!match) return null;
  const colors = match[2]
    .split(",")
    .map((stop) => stop.trim().match(/^(#[0-9a-f]{6})(?:\s+\d+(?:\.\d+)?%)?$/i)?.[1]?.toLowerCase())
    .filter((color): color is string => Boolean(color));
  if (colors.length === 0) return null;
  const editorColors = colors.every((color) => color === colors[0]) ? [colors[0]] : colors;

  return {
    angle: clampGradientAngle(Number(match[1])),
    colors: editorColors,
  };
}

function safeHexColor(value: string | null | undefined, fallback: string): string {
  const current = value?.trim();
  if (current && hexColorPattern.test(current)) return current.toLowerCase();
  return hexColorPattern.test(fallback) ? fallback.toLowerCase() : defaultColorFallback;
}

function clampGradientAngle(value: number): number {
  return Math.min(360, Math.max(0, Math.round(value || 0)));
}

function safeColorStops(colors: string[] | null | undefined, fallback: string): string[] {
  const sanitized = (colors ?? [])
    .map((color) => safeHexColor(color, ""))
    .filter(Boolean);
  return sanitized.length ? sanitized : [safeHexColor(fallback, defaultColorFallback)];
}

function gradientStops(colors: string[]): string {
  if (colors.length === 1) return `${colors[0]} 0%, ${colors[0]} 100%`;
  return colors
    .map((color, index) => `${color} ${Math.round((index / (colors.length - 1)) * 100)}%`)
    .join(", ");
}

function marqueePalette(colors: string[]): string[] {
  const safeColors = safeColorStops(colors, defaultRunningBubbleColors.border);
  return [0, 1, 2, 3, 4].map((index) => safeColors[index % safeColors.length]);
}
