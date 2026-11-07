import { expect, it } from 'vitest';
import { initialSurface } from './surface';
import { petSizing } from './sizing';

it('grows gently across screens and stays compact on a 4K logical work area', () => {
  const small = petSizing({ ...initialSurface, availableWidth: 1024, availableHeight: 640 });
  const normal = petSizing({ ...initialSurface, availableWidth: 1440, availableHeight: 860 });
  const large = petSizing({ ...initialSurface, availableWidth: 3840, availableHeight: 2160 });
  expect(normal.expandedWidth).toBeGreaterThan(small.expandedWidth);
  expect(large.expandedWidth).toBeGreaterThan(normal.expandedWidth);
  expect(large.expandedWidth).toBeLessThanOrEqual(640);
  expect(large.expandedHeight).toBeLessThanOrEqual(440);
  expect(large.desktopWidth).toBeLessThanOrEqual(360);
  expect(large.collapsedWidth).toBeLessThanOrEqual(260);
  expect(large.headerHeight).toBeLessThanOrEqual(44);
  expect(large.expandedWidth / 3840).toBeLessThan(normal.expandedWidth / 1440);
});

it('keeps tiny displays bounded and reserves actual rail space before scrolling', () => {
  const size = petSizing({ ...initialSurface, availableWidth: 280, availableHeight: 340 }, 4, 140);
  expect(size.expandedWidth).toBeLessThanOrEqual(280);
  expect(size.desktopWidth).toBeLessThanOrEqual(280);
  expect(size.stageHeight).toBeGreaterThanOrEqual(140);
  expect(size.stageHeight + size.desktopListHeight + 30).toBeLessThanOrEqual(340);
  expect(size.portraitWidth * 2 / 3).toBeLessThanOrEqual(size.headerHeight - 11);
});

it('fits ultrawide characters by height and keeps user pet scale meaningful', () => {
  const surface = { ...initialSurface, availableWidth: 3440, availableHeight: 720 };
  const small = petSizing(surface, 2), large = petSizing(surface, 4);
  expect(large.petWidth).toBeGreaterThan(small.petWidth);
  expect(large.petWidth).toBeLessThan(720 * .2);
  expect(large.stageHeight + large.desktopListHeight + 30).toBeLessThanOrEqual(720 * .55);
});

it('uses safe startup defaults for missing or nonfinite display data', () => {
  const size = petSizing({ ...initialSurface, availableWidth: NaN, availableHeight: Infinity }, NaN);
  expect(Object.values(size).every(value => Number.isFinite(value) && value > 0)).toBe(true);
});
