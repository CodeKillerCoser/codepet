// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DockPanelMotion } from './dockPanelMotion';
import type { PetSurfaceController, SurfaceLayout } from './surface';

let frame: HTMLElement, motion: DockPanelMotion;
let height: number;
let native: { x: number; y: number; width: number; height: number };
let surface: Pick<PetSurfaceController, 'measure' | 'plan' | 'beginFrame' | 'endFrame' | 'commit' | 'pauseLayout' | 'resumeLayout'>;
const failed = vi.fn();
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
};
beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now());
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => setTimeout(() => fn(performance.now()), 16));
  vi.stubGlobal('cancelAnimationFrame', clearTimeout);
  height = 180;
  native = { x: 620, y: 0, width: 200, height: 24 };
  frame = document.createElement('div');
  // Deliberately arbitrary contents: the driver must not depend on task classes.
  frame.innerHTML = '<input value="keep my focus"><article>Any slot content</article>';
  frame.getBoundingClientRect = () => ({ width: 200, height: 24 }) as DOMRect;
  document.body.append(frame);
  surface = {
    measure: vi.fn(() => ({ width: frame.dataset.measure === 'expanded' ? 440 : 200,
      height: frame.dataset.measure === 'expanded' ? height : 24,
      petWidth: 24, petHeight: 24, anchor: { x: 20, y: 12 }, revision: 1 })),
    plan: vi.fn(async (layout: SurfaceLayout) => ({ from: { ...native },
      to: { x: 720 - layout.width / 2, y: 0, width: layout.width, height: layout.height }, topInset: 0 })),
    commit: vi.fn(async layout => { native = { x: 720 - layout.width / 2, y: 0, width: layout.width, height: layout.height }; }),
    beginFrame: vi.fn(async () => 1), endFrame: vi.fn(async () => {}),
    pauseLayout: vi.fn(), resumeLayout: vi.fn(),
  };
  motion = new DockPanelMotion(frame, surface as PetSurfaceController, failed);
});
afterEach(() => { motion.dispose(); document.body.replaceChildren(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('preserves slot identity and focus, then remeasures content updated during expansion', async () => {
  const input = frame.querySelector('input')!;
  input.focus();
  const opening = motion.request(true);
  await vi.advanceTimersByTimeAsync(100);
  height = 300;
  frame.querySelector('article')!.textContent = 'A longer update';
  motion.refresh();
  await vi.advanceTimersByTimeAsync(700); await opening;
  expect(native.height).toBe(300);
  expect(frame.style.height).toBe('300px');
  expect(frame.querySelector('input')).toBe(input);
  expect(document.activeElement).toBe(input);
  expect(frame.dataset.phase).toBe('open');
  expect(frame.hasAttribute('data-morphing')).toBe(false);
  expect(failed).not.toHaveBeenCalled();
});

it('coalesces requests during the native handshake to the newest target', async () => {
  const cover = deferred<number>();
  vi.mocked(surface.beginFrame).mockReturnValueOnce(cover.promise);
  const opening = motion.request(true);
  await motion.request(false);
  cover.resolve(9);
  await vi.advanceTimersByTimeAsync(700); await opening;
  expect(native.width).toBe(200);
  expect(frame.dataset.phase).toBe('closed');
  expect(surface.endFrame).toHaveBeenCalledWith(9);
});

it('continues a reversal from the current visual frame', async () => {
  void motion.request(true);
  await vi.advanceTimersByTimeAsync(110);
  const width = frame.style.width;
  void motion.request(false);
  expect(frame.style.width).toBe(width);
  await vi.advanceTimersByTimeAsync(600);
  expect(native.width).toBe(200);
  expect(frame.dataset.phase).toBe('closed');
});

it('aborts a cover delivered after unmount and never commits its layout', async () => {
  const cover = deferred<number>();
  vi.mocked(surface.beginFrame).mockReturnValueOnce(cover.promise);
  const opening = motion.request(true);
  motion.dispose(); cover.resolve(7); await opening;
  expect(surface.endFrame).toHaveBeenCalledWith(7, true);
  expect(surface.commit).not.toHaveBeenCalled();
});

it('releases the native cover and resumes measurement if the frame commit fails', async () => {
  const error = new Error('native layout failed');
  vi.mocked(surface.commit).mockRejectedValueOnce(error);
  await motion.request(true);
  expect(surface.endFrame).toHaveBeenCalledWith(1, true);
  expect(surface.resumeLayout).toHaveBeenCalled();
  expect(frame.hasAttribute('data-morphing')).toBe(false);
  expect(failed).toHaveBeenCalledWith(error);
  const closing = motion.request(false); await vi.advanceTimersByTimeAsync(500); await closing;
});
