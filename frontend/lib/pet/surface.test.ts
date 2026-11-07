// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { initialSurface, measureSurface, PetSurfaceController, type SurfaceView } from './surface';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
let send: (value: SurfaceView) => void;
let measure: () => void;
let controller: PetSurfaceController;
let content: HTMLElement;
const unlisten = vi.fn(), disconnect = vi.fn();
const view = { ...initialSurface, revision: 1, canDock: true };
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('ResizeObserver', class { constructor(fn: () => void) { measure = fn; } observe() {} disconnect = disconnect; });
  vi.mocked(listen).mockImplementation(async (_event, handler: any) => { send = payload => handler({ payload }); return unlisten; });
  vi.mocked(invoke).mockResolvedValue(view);
  content = document.createElement('div');
  content.innerHTML = '<div class="pet-avatar"></div>';
  content.getBoundingClientRect = () => ({ left: 5, top: 40, width: 320, height: 200 }) as DOMRect;
  content.firstElementChild!.getBoundingClientRect = () => ({ left: 205, top: 140, width: 80, height: 90 }) as DOMRect;
});
afterEach(() => { controller?.dispose(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.resetAllMocks(); });
it('measures the character anchor relative to content rather than viewport or window edge', () => {
  expect(measureSurface(content, 7)).toEqual({ width: 320, height: 200, petWidth: 80, petHeight: 90, anchor: { x: 240, y: 145 }, revision: 7 });
});
it('ignores a stale layout reply after a dock event and remeasures the new epoch', async () => {
  const changed = vi.fn(); controller = new PetSurfaceController(content, changed, error => { throw error; });
  await controller.start();
  let finish!: (value: SurfaceView) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve as typeof finish; }));
  await vi.advanceTimersByTimeAsync(65);
  send({ ...view, revision: 2, mode: 'docked' }); measure();
  finish(view); await vi.advanceTimersByTimeAsync(130);
  expect(changed.mock.calls.at(-1)![0].mode).toBe('docked');
  const calls = vi.mocked(invoke).mock.calls.filter(([name]) => name === 'pet_surface_layout');
  expect((calls.at(-1)![1] as any).layout.revision).toBe(2);
});
it('finishes a fast native drag even when mouse-up arrives before the start reply', async () => {
  controller = new PetSurfaceController(content, vi.fn(), vi.fn()); await controller.start();
  let start!: (value: SurfaceView) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { start = resolve as typeof start; }));
  const done = vi.fn(); const dragging = controller.drag().then(done);
  send({ ...view, revision: 2, dragging: true });
  send({ ...view, revision: 3, dragging: false, mode: 'docked' });
  start({ ...view, revision: 2, dragging: true });
  await dragging; expect(done).toHaveBeenCalledOnce();
});
it('disconnects measurement and events and does not schedule layout after disposal', async () => {
  controller = new PetSurfaceController(content, vi.fn(), vi.fn()); await controller.start(); controller.dispose();
  measure(); await vi.advanceTimersByTimeAsync(200);
  expect(unlisten).toHaveBeenCalledOnce(); expect(disconnect).toHaveBeenCalledOnce();
  expect(vi.mocked(invoke).mock.calls.map(c => c[0])).toEqual(['pet_surface_state']);
});
