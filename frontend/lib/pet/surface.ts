import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

export interface SurfaceView {
  revision: number;
  mode: 'floating' | 'docked';
  dragging: boolean;
  snapCandidate: boolean;
  canDock: boolean;
  availableHeight: number;
  availableWidth?: number;
  nativeDrag: boolean;
  physicalNotch: boolean;
}
export const initialSurface: SurfaceView = {
  revision: 0, mode: 'floating', dragging: false, snapCandidate: false,
  canDock: false, availableHeight: 700, nativeDrag: true, physicalNotch: false,
};
export interface SurfaceLayout { width: number; height: number; petWidth: number; petHeight: number; anchor: { x: number; y: number }; revision: number }

export function measureSurface(content: HTMLElement, revision: number): SurfaceLayout | null {
  const pet = content.querySelector<HTMLElement>('.pet-avatar');
  if (!pet) return null;
  const bounds = content.getBoundingClientRect(), actor = pet.getBoundingClientRect();
  if (bounds.height <= 0 || actor.width <= 0) return null;
  return {
    width: Math.ceil(bounds.width), height: Math.ceil(bounds.height), petWidth: actor.width, petHeight: actor.height,
    anchor: { x: actor.left - bounds.left + actor.width / 2, y: actor.top - bounds.top + actor.height / 2 },
    revision,
  };
}

/** Owns the frontend/native layout handshake. No component writes the window frame. */
export class PetSurfaceController {
  private view = initialSurface;
  private disposed = false;
  private unlisten?: UnlistenFn;
  private observer?: ResizeObserver;
  private timer?: ReturnType<typeof setTimeout>;
  private inFlight = false;
  private pending = false;
  private dragFinished?: () => void;
  constructor(private content: HTMLElement, private changed: (view: SurfaceView) => void, private failed: (error: unknown) => void) {}

  async start() {
    const unlisten = await listen<SurfaceView>('pet-surface-changed', event => this.accept(event.payload));
    if (this.disposed) { unlisten(); return; }
    this.unlisten = unlisten;
    this.observer = new ResizeObserver(() => this.schedule());
    this.observer.observe(this.content);
    this.accept(await invoke<SurfaceView>('pet_surface_state'));
    this.schedule();
  }
  private accept(next: SurfaceView) {
    if (this.disposed || next.revision < this.view.revision) return;
    if (this.view.dragging && !next.dragging) { this.dragFinished?.(); this.dragFinished = undefined; }
    if (JSON.stringify(next) !== JSON.stringify(this.view)) {
      this.view = next; this.changed(next); this.schedule();
    }
  }
  schedule() {
    if (this.disposed || !this.view.revision) return;
    this.pending = true;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), 60);
  }
  private async flush() {
    if (this.disposed || this.inFlight || !this.pending) return;
    this.pending = false; this.inFlight = true;
    try {
      const layout = measureSurface(this.content, this.view.revision);
      if (layout) this.accept(await invoke<SurfaceView>('pet_surface_layout', { layout }));
    } catch (error) { this.failed(error); }
    finally { this.inFlight = false; if (this.pending) this.schedule(); }
  }
  async setMode(mode: SurfaceView['mode']) {
    this.accept(await invoke<SurfaceView>('pet_surface_mode', { mode }));
  }
  async drag() {
    if (!this.view.nativeDrag) {
      await getCurrentWindow().startDragging();
      this.schedule();
      return;
    }
    // Subscribe before starting: a quick release may arrive before the invoke response.
    let finish!: () => void;
    const ended = new Promise<void>(resolve => { finish = resolve; });
    this.dragFinished = finish;
    try {
      const next = await invoke<SurfaceView>('pet_surface_drag', { cancel: false });
      this.accept(next);
      if (!next.dragging || !this.view.dragging) finish();
      await ended;
    } finally { if (this.dragFinished === finish) this.dragFinished = undefined; }
  }
  async cancelDrag() { this.accept(await invoke<SurfaceView>('pet_surface_drag', { cancel: true })); }
  dispose() {
    this.disposed = true; clearTimeout(this.timer);
    this.observer?.disconnect(); this.unlisten?.(); this.dragFinished?.();
  }
}
