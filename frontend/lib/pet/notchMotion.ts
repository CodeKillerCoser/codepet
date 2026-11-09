import { tick } from 'svelte';
import type { PetSurfaceController } from './surface';

const shared = ['.pet-avatar', '.status-dot', '.latest-status', '.latest-title', '.task-count', '.chevron', '.panel-actions'];
const duration = 260;
const easing = 'cubic-bezier(.3,0,.2,1)';
type Box = { x: number; y: number; width: number; height: number; opacity: number };
const box = (el: Element): Box => {
  const r = el.getBoundingClientRect();
  return { x: r.x, y: r.y, width: r.width, height: r.height, opacity: Number(getComputedStyle(el).opacity) };
};

/** The top panel's one transition coordinator. Geometry still belongs to Surface. */
export class NotchMotion {
  private sequence = 0;
  private animations: Animation[] = [];
  private ghost?: HTMLElement;
  private nativeFrame?: number;
  private preparing = false;
  private pending?: { expanded: boolean; commitUI: () => void };
  private finishPreparing() {
    this.preparing = false;
    const next = this.pending; this.pending = undefined;
    if (next) { void this.transition(next.expanded, next.commitUI); return true; }
    return false;
  }
  private disposed = false;
  constructor(private content: HTMLElement, private surface: PetSurfaceController, private failed: (error: unknown) => void) {}

  private snapshot(root: HTMLElement) {
    const ghost = root.cloneNode(true) as HTMLElement;
    ghost.setAttribute('aria-hidden', 'true'); ghost.inert = true;
    ghost.dataset.motionGhost = '';
    Object.assign(ghost.style, { position: 'fixed', left: `${root.getBoundingClientRect().left}px`, top: '0px', width: `${root.getBoundingClientRect().width}px`, maxWidth: 'none', pointerEvents: 'none', zIndex: '1000' });
    // cloneNode doesn't copy in-flight Web Animations. Freeze their visible styles.
    for (const selector of ['.pet-content', '.notch-backdrop', '.notch-drag-region', '.notch-details', ...shared]) {
      const source = root.querySelector(selector), copy = ghost.querySelector<HTMLElement>(selector);
      if (!source || !copy) continue;
      const css = getComputedStyle(source);
      for (const property of ['transform', 'opacity', 'clip-path', 'width', 'height', 'left', 'top', 'border-radius']) {
        copy.style.setProperty(property, css.getPropertyValue(property));
      }
    }
    document.body.append(ghost);
    return ghost;
  }
  private stopAnimations() { this.animations.forEach(a => a.cancel()); this.animations = []; }
  cancel() {
    this.pending = undefined; this.preparing = false;
    this.sequence++; this.stopAnimations(); this.ghost?.remove(); this.ghost = undefined;
    if (this.nativeFrame !== undefined) {
      void this.surface.endFrame(this.nativeFrame, true).catch(this.failed);
      this.nativeFrame = undefined;
    }
    this.content.style.removeProperty('opacity'); this.content.style.removeProperty('left');
    delete this.content.dataset.motionPreparing;
    delete this.content.dataset.morphing;
    this.surface.resumeLayout();
  }
  dispose() { this.cancel(); this.disposed = true; }

  async transition(expanded: boolean, commitUI: () => void) {
    if (this.disposed) return;
    // Do not sample halfway through a native viewport handoff. Its source
    // pixels still belong to the old frame, while DOM may already use the new one.
    if (this.preparing) { this.pending = { expanded, commitUI }; return; }
    this.preparing = true;
    const id = ++this.sequence;
    this.animations.forEach(a => a.pause());
    // A reversal can arrive while the native frame is still being prepared.
    // Capture what is visible, not the hidden destination layout.
    const visual = this.content.dataset.motionPreparing !== undefined && this.ghost ? this.ghost : this.content.parentElement!;
    const oldBoxes = new Map<string, Box>();
    for (const selector of shared) {
      const el = visual.querySelector(selector);
      if (el && el.getBoundingClientRect().width) oldBoxes.set(selector, box(el));
    }
    const oldBackground = box(visual.querySelector('.notch-backdrop')!);
    const ghost = this.snapshot(visual);
    const ghostLeft = ghost.getBoundingClientRect().left;
    this.stopAnimations(); this.ghost?.remove(); this.ghost = ghost;
    this.surface.pauseLayout();
    this.content.dataset.morphing = '';
    // Opacity preserves keyboard focus and hover while the snapshot is visible.
    // visibility:hidden would blur the summary and schedule an unwanted collapse.
    this.content.style.opacity = '0';
    this.content.dataset.motionPreparing = '';
    this.content.style.left = '0px';
    commitUI();
    const alive = () => !this.disposed && id === this.sequence;
    try {
      await tick();
      if (!alive()) return;
      let token = await this.surface.beginFrame();
      if (!alive()) { await this.surface.endFrame(token, true); return; }
      this.nativeFrame = token;
      let layout = this.surface.measure();
      if (!layout) throw new Error('Cannot measure top panel transition');
      let plan = await this.surface.plan(layout);
      if (!alive()) return;
      const panel = this.content.querySelector<HTMLElement>('.notch-panel')!;
      // Preview returns actual clamped geometry, including crossing the notch's safe area.
      if (Number.parseFloat(getComputedStyle(panel).getPropertyValue('--notch-top-inset')) !== plan.topInset) {
        panel.style.setProperty('--notch-top-inset', `${plan.topInset}px`);
        if (!plan.topInset) panel.style.setProperty('--notch-min-width', '0px');
        layout = this.surface.measure()!;
        plan = await this.surface.plan(layout);
        if (!alive()) return;
      }
      let screenOffset = 0;
      if (expanded && plan.to.width > plan.from.width) {
        // Before resize this is zero. During native resize CSS automatically preserves
        // the old screen position, including panels clamped at either screen edge.
        const fraction = (plan.from.x - plan.to.x) / (plan.to.width - plan.from.width);
        ghost.style.left = `calc(${ghostLeft}px + (100vw - ${plan.from.width}px) * ${fraction})`;
        await this.surface.commit(layout);
        if (!alive()) return;
        await tick();
        // A safe-area change may invalidate the first layout epoch.
        const settled = this.surface.measure();
        if (settled && settled.revision !== layout.revision) await this.surface.commit(settled);
        if (!alive()) return;
        screenOffset = plan.from.x - plan.to.x;
      } else if (!expanded) {
        // Keep the larger native canvas until collapse finishes. This formula becomes
        // zero when native resizes to the compact frame, so the final frame cannot jump.
        const difference = plan.from.width - plan.to.width;
        const fraction = difference ? (plan.to.x - plan.from.x) / difference : 0;
        this.content.style.left = `calc((100vw - ${plan.to.width}px) * ${fraction})`;
      }
      const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
      const ms = reduced ? 0 : duration;
      const animate = (el: Element, frames: Keyframe[], delay = 0, time = ms) => {
        const a = el.animate(frames, { duration: time, delay: reduced ? 0 : delay, easing, fill: 'both' });
        // Native frame changes and WebKit presentation happen on different clocks.
        // Hold the initial visual until the native cover can be safely removed.
        a.pause(); a.currentTime = 0;
        this.animations.push(a); return a;
      };
      const background = this.content.querySelector<HTMLElement>('.notch-backdrop')!;
      const endBackground = box(background);
      const startBackground = { ...oldBackground, x: oldBackground.x + screenOffset };
      // Reveal against the moving surface, not the final panel's larger rectangle.
      // Negative insets retain the outgoing list while a compact layout is underneath.
      const clip = (root: Box, r: Box) => `inset(${r.y - root.y}px ${root.x + root.width - r.x - r.width}px ${root.y + root.height - r.y - r.height}px ${r.x - root.x}px)`;
      // Clip foreground only; the backdrop's outer shoulders remain visible.
      for (const selector of ['.notch-details']) {
        const el = this.content.querySelector(selector);
        if (el) { const bounds = box(el); animate(el, [{ clipPath: clip(bounds, startBackground) }, { clipPath: clip(bounds, endBackground) }]); }
      }
      animate(background, [
        { left: `${oldBackground.x + screenOffset - endBackground.x}px`, top: `${oldBackground.y - endBackground.y}px`, width: `${oldBackground.width}px`, height: `${oldBackground.height}px` },
        { left: '0px', top: '0px', width: `${endBackground.width}px`, height: `${endBackground.height}px` },
      ]);
      for (const selector of shared) {
        const el = this.content.querySelector<HTMLElement>(selector);
        if (!el || !el.getBoundingClientRect().width) continue;
        const old = oldBoxes.get(selector), end = box(el);
        if (old) {
          const dx = old.x + screenOffset - end.x, dy = old.y - end.y;
          // Use the same layout/paint clock as the background's left/width.
          // WKWebView can present compositor transforms ahead of layout animation,
          // briefly moving the avatar outside the still-narrow background.
          animate(el, [
            { left: `${dx}px`, top: `${dy}px`, opacity: old.opacity,
              ...(selector === '.latest-title' ? { clipPath: `inset(0 ${Math.max(0, end.width - old.width)}px 0 0)` } : {}) },
            { left: '0px', top: '0px', opacity: 1,
              ...(selector === '.latest-title' ? { clipPath: 'inset(0 0 0 0)' } : {}) },
          ]);
        } else animate(el, [{ opacity: 0 }, { opacity: 1 }], 190, reduced ? 0 : 70);
      }
      if (expanded) {
        const details = this.content.querySelector('.notch-details');
        if (details) animate(details, [{ opacity: 0, transform: 'translateY(-5px)' }, { opacity: 1, transform: 'translateY(0)' }], 80, reduced ? 0 : 180);
        ghost.remove(); this.ghost = undefined;
      } else {
        // Keep only the outgoing list while the shared header moves to its compact layout.
        ghost.querySelector<HTMLElement>('.notch-drag-region')!.style.visibility = 'hidden';
        ghost.querySelector<HTMLElement>('.notch-backdrop')!.style.visibility = 'hidden';
        const ghostBox = box(ghost);
        animate(ghost, [{ clipPath: clip(ghostBox, startBackground) }, { clipPath: clip(ghostBox, endBackground) }]);
        animate(ghost, [{ opacity: 1 }, { opacity: 0 }], 0, reduced ? 0 : 100);
      }
      this.content.style.removeProperty('opacity'); delete this.content.dataset.motionPreparing;
      await tick();
      await this.surface.endFrame(token);
      if (!alive()) return;
      this.nativeFrame = undefined;
      if (this.finishPreparing()) return;
      this.animations.forEach(a => a.play());
      await Promise.all(this.animations.map(a => a.finished.catch(() => {})));
      if (!alive()) return;
      if (!expanded) {
        this.preparing = true;
        token = await this.surface.beginFrame();
        if (!alive()) { await this.surface.endFrame(token, true); return; }
        this.nativeFrame = token;
        await this.surface.commit(layout);
        if (!alive()) return;
      }
      this.stopAnimations(); ghost.remove(); this.ghost = undefined;
      this.content.style.removeProperty('left'); delete this.content.dataset.morphing;
      if (!expanded) {
        await tick();
        await this.surface.endFrame(token);
        if (!alive()) return;
        this.nativeFrame = undefined;
      }
      this.surface.resumeLayout();
      this.finishPreparing();
    } catch (error) {
      if (alive()) { this.cancel(); this.failed(error); }
    }
  }
}
