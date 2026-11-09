import type { PetSurfaceController } from './surface';

type Visual = { x: number; width: number; height: number; progress: number; inset: number };
const duration = 260;
const closeEnough = (a: Visual, b: Visual) => Math.abs(a.width - b.width) < 1 && Math.abs(a.height - b.height) < 1 && a.inset === b.inset && a.progress === b.progress;

/** Private to DockPanel. Only touches the shell's own frame, never slot contents. */
export class DockPanelMotion {
  private visual: Visual;
  private desired = false;
  private preparing = false;
  private running = false;
  private dirty = false;
  private disposed = false;
  private sequence = 0;
  private token?: number;
  private stopAnimation?: () => void;

  constructor(private frame: HTMLElement, private surface: PetSurfaceController, private failed: (error: unknown) => void) {
    const bounds = frame.getBoundingClientRect();
    this.visual = { x: 0, width: bounds.width, height: bounds.height, progress: 0, inset: this.safeInset() };
    this.paint(this.visual);
    frame.dataset.phase = 'closed';
    frame.dataset.ready = '';
  }

  private safeInset() { return parseFloat(getComputedStyle(this.frame).getPropertyValue('--dock-safe-inset')) || 0; }
  private paint(value: Visual) {
    this.visual = value;
    const style = this.frame.style;
    style.width = `${value.width}px`; style.height = `${value.height}px`; style.left = `${value.x}px`;
    style.setProperty('--dock-progress', String(value.progress));
    style.setProperty('--dock-inset', `${value.inset}px`);
  }

  private measure(expanded: boolean, inset = this.safeInset()) {
    // Synchronous endpoint measurement: CSS temporarily lays out the same nodes.
    // Restore before yielding; neither a business DOM clone nor an extra paint.
    this.frame.dataset.measure = expanded ? 'expanded' : 'collapsed';
    this.frame.style.setProperty('--dock-inset', `${inset}px`);
    let layout;
    try { layout = this.surface.measure(); }
    finally { delete this.frame.dataset.measure; this.paint(this.visual); }
    if (!layout || !layout.revision) return null;
    return { layout, visual: { x: 0, width: layout.width, height: layout.height, progress: Number(expanded), inset } };
  }

  refresh() {
    if (this.disposed) return;
    if (this.running) { this.dirty = true; return; }
    const target = this.measure(this.desired);
    if (target && !closeEnough(this.visual, target.visual)) void this.request(this.desired);
  }

  async request(expanded: boolean) {
    this.desired = expanded;
    if (this.disposed || this.preparing) { this.dirty = true; return; }
    const target = this.measure(expanded);
    if (!target) return;
    const id = ++this.sequence;
    this.stopAnimation?.();
    this.preparing = true; this.running = true; this.dirty = false;
    this.frame.dataset.morphing = '';
    this.frame.dataset.phase = expanded ? 'opening' : 'closing';
    this.surface.pauseLayout();
    const alive = () => !this.disposed && id === this.sequence;
    let layout = target.layout;
    try {
      let token = await this.surface.beginFrame();
      if (!alive()) { await this.surface.endFrame(token, true); return; }
      this.token = token;
      let plan = await this.surface.plan(layout);
      if (!alive()) return;
      if (plan.topInset !== target.visual.inset) {
        const adjusted = this.measure(expanded, plan.topInset);
        if (!adjusted) throw new Error('Cannot measure dock panel');
        layout = adjusted.layout; target.visual = adjusted.visual;
        plan = await this.surface.plan(layout);
        if (!alive()) return;
      }
      const grows = plan.to.width > plan.from.width || plan.to.height > plan.from.height;
      if (grows) {
        const offset = plan.from.x - plan.to.x;
        // The native cover owns presentation while the viewport changes.
        await this.surface.commit(layout);
        if (!alive()) return;
        this.paint({ ...this.visual, x: this.visual.x + offset });
      }
      target.visual.x = grows ? 0 : plan.to.x - plan.from.x;
      await this.surface.endFrame(token);
      if (!alive()) return;
      this.token = undefined; this.preparing = false;
      if (this.desired !== expanded) { void this.request(this.desired); return; }
      await this.animate(target.visual);
      if (!alive()) return;
      if (!grows) {
        this.preparing = true;
        token = await this.surface.beginFrame();
        if (!alive()) { await this.surface.endFrame(token, true); return; }
        this.token = token;
        await this.surface.commit(layout);
        if (!alive()) return;
        this.paint({ ...target.visual, x: 0 });
        await this.surface.endFrame(token);
        if (!alive()) return;
        this.token = undefined;
      }
      this.paint({ ...target.visual, x: 0 });
      this.preparing = false; this.running = false;
      this.frame.dataset.phase = expanded ? 'open' : 'closed';
      delete this.frame.dataset.morphing;
      this.surface.resumeLayout();
      if (this.desired !== expanded) void this.request(this.desired);
      else if (this.dirty) { this.dirty = false; this.refresh(); }
    } catch (error) {
      if (!alive()) return;
      await this.releaseCover();
      if (!alive()) return;
      this.paint({ ...target.visual, x: 0 });
      this.preparing = false; this.running = false;
      this.frame.dataset.phase = expanded ? 'open' : 'closed';
      delete this.frame.dataset.morphing;
      this.surface.resumeLayout();
      this.failed(error);
      if (this.desired !== expanded) void this.request(this.desired);
    }
  }

  private animate(target: Visual) {
    const start = { ...this.visual };
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) { this.paint(target); return Promise.resolve(); }
    return new Promise<void>(resolve => {
      let raf = 0;
      const started = performance.now();
      const stop = () => { cancelAnimationFrame(raf); if (this.stopAnimation === stop) this.stopAnimation = undefined; resolve(); };
      this.stopAnimation = stop;
      const step = (now: number) => {
        const t = Math.min(1, (now - started) / duration), p = 1 - (1 - t) ** 3;
        const value = {} as Visual;
        for (const key of ['x', 'width', 'height', 'progress', 'inset'] as const) value[key] = start[key] + (target[key] - start[key]) * p;
        this.paint(value);
        if (t < 1) raf = requestAnimationFrame(step); else stop();
      };
      raf = requestAnimationFrame(step);
    });
  }

  private async releaseCover() {
    if (this.token === undefined) return;
    const token = this.token; this.token = undefined;
    try { await this.surface.endFrame(token, true); } catch (error) { this.failed(error); }
  }
  dispose() {
    this.disposed = true; this.sequence++;
    this.stopAnimation?.();
    void this.releaseCover();
    this.surface.resumeLayout();
  }
}
