import type { SurfaceView } from './surface';

// Dimensions are logical screen units supplied by the native display owner,
// never the pet webview viewport (which itself changes after measurement).
export function petSizing(surface: SurfaceView, scale = 3, railHeight = 56) {
  const positive = (value: number | undefined, fallback: number) =>
    value && Number.isFinite(value) && value > 0 ? value : fallback;
  const width = positive(surface.availableWidth, 1440);
  const height = positive(surface.availableHeight, 860);
  // Grow more slowly than the display, with compact bounds on large screens.
  const widthScale = Math.sqrt(width / 1440);
  const heightScale = Math.sqrt(height / 860);
  const bounded = (value: number, min: number, max: number) => Math.round(Math.min(max, Math.max(min, value)));
  const fitWidth = (base: number, min: number, max: number) => Math.min(width, bounded(base * widthScale, min, max));
  const headerHeight = Math.min(height, bounded(24 * heightScale, 22, 26));
  const desktopWidth = fitWidth(280, 240, 360);
  const petWidth = Math.max(1, Math.round(Math.min(desktopWidth - 60,
    bounded(80 * Math.min(widthScale, heightScale), 64, 104) * Math.min(4, Math.max(2, Number.isFinite(scale) ? scale : 3)) / 3)));
  const stageHeight = Math.max(Math.round(petWidth * 208 / 192), railHeight);
  return {
    collapsedWidth: fitWidth(200, 160, 260),
    expandedWidth: fitWidth(440, 340, 640),
    expandedHeight: Math.min(height, bounded(340 * heightScale, headerHeight + 80, 440)),
    headerHeight,
    portraitWidth: (headerHeight - 4) * 1.5,
    desktopWidth,
    petWidth,
    stageHeight,
    // Only the list scrolls. Reserve the actual character/action rail and padding.
    desktopListHeight: Math.max(0, Math.min(height, bounded(360 * heightScale, 180, 440)) - stageHeight - 30),
  };
}
