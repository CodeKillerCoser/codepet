/** Plane Cards: positions orbit in Y/depth; the planes never rotate. */
export type PlaneStackFrame = { position: number; moving: boolean };
const wrap = (value: number, count: number) => ((value % count) + count) % count;
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

export function planeStackCards<T>(items: T[], position: number) {
  const count = items.length;
  if (!count) return [];
  if (count === 1) return [{ item: items[0], index: 0, front: true, y: 0, scale: 1, opacity: 1, ink: 1, layer: 2000, depth: 0 }];
  const nearest = Math.round(position);
  const front = wrap(nearest, count);
  // Zero at every resting slot, one halfway through a turn. Both endpoints have
  // zero slope, including the nearest-slot change, so settling never snaps.
  const orbit = Math.sin(Math.PI * (position - Math.floor(position))) ** 2;
  const blend = (compact: number, rotating: number) => compact + (rotating - compact) * orbit;
  // A virtual three-slot orbit bounds DOM work independently of history length.
  // Cards enter/leave at its invisible rear seam. Two cards use opposite slots.
  const candidates = count === 2 ? [Math.floor(position), Math.floor(position) + 1]
    : Array.from({ length: 4 }, (_, i) => Math.ceil(position - 1.5) + i);
  return candidates.filter(slot => count === 2 || Math.abs(slot - position) < 1.5)
    .map(slot => {
      const index = wrap(slot, count);
      const offset = slot - position;
      const angle = offset * (count === 2 ? Math.PI : Math.PI * 2 / 3);
      const depth = 1 - Math.cos(angle);
      // Preserve the original stack's 600px perspective / 18px layer spacing.
      // The orbit changes position, not the established 94.3–100% width range.
      const compactDepth = wrap(slot - nearest, Math.min(count, 3));
      const compactScale = 600 / (600 + 18 * compactDepth);
      const scale = blend(compactScale, 600 / (600 + 18 * depth));
      // Match the old top-origin perspective stack exactly at rest. The orbit
      // stays within its existing 22px footprint instead of reserving empty space.
      const y = blend(11 * compactDepth * compactScale, 11 + 11 * Math.sin(angle));
      const opacity = count === 2 ? 1 : clamp((1.5 - Math.abs(offset)) * 2, 0, 1);
      const ink = clamp((Math.cos(angle) + .25) / 1.25, 0, 1);
      return { item: items[index], index, front: index === front, y, scale, opacity,
        ink: ink * ink * (3 - 2 * ink), layer: Math.round(blend(2000 - compactDepth * 750, (2 - depth) * 1000)), depth: blend(compactDepth, depth) };
    }).sort((a, b) => Number(b.front) - Number(a.front) || b.layer - a.layer || a.index - b.index);
}

/** Owns only presentation progress; no task data, window sizing or subscriptions. */
export class PlaneStackMotion {
  private ids: string[] = [];
  private position = 0;
  private target = 0;
  private velocity = 0;
  private token: number | undefined;
  private last = 0;
  private enabled = true;
  private reduced = false;
  constructor(private changed: (frame: PlaneStackFrame) => void) {}
  sync(ids: string[], enabled: boolean, reduced: boolean) {
    if (ids.length !== this.ids.length || ids.some((id, i) => id !== this.ids[i])) {
      const selected = this.ids[wrap(Math.round(this.position), this.ids.length)];
      this.stop();
      this.ids = ids;
      this.position = this.target = Math.max(0, ids.indexOf(selected));
      this.emit();
    }
    if ((!enabled && this.enabled) || (reduced && !this.reduced)) {
      this.stop();
      this.position = this.target = wrap(Math.round(this.position), ids.length || 1);
      this.emit();
    }
    this.enabled = enabled;
    this.reduced = reduced;
  }
  step(direction: number) {
    if (!this.enabled || this.ids.length < 2) return;
    this.target = clamp(this.target + Math.sign(direction), Math.floor(this.position) - 2, Math.ceil(this.position) + 2);
    if (this.reduced) {
      this.position = this.target = wrap(this.target, this.ids.length);
      this.emit();
      return;
    }
    if (this.token === undefined) {
      this.last = performance.now();
      this.token = requestAnimationFrame(this.tick);
      this.emit();
    }
  }
  private tick = (now: number) => {
    const dt = Math.min((now - this.last) / 1000, .064);
    this.last = now;
    // Exact critically damped spring step: retargeting preserves position/velocity.
    // Opposite two-card slots travel a wider arc, so keep their angular speed gentle.
    const omega = this.ids.length === 2 ? 12 : 16, offset = this.position - this.target;
    const decay = Math.exp(-omega * dt), term = this.velocity + omega * offset;
    this.position = this.target + (offset + term * dt) * decay;
    this.velocity = (this.velocity - omega * term * dt) * decay;
    if (Math.abs(this.target - this.position) < .0005 && Math.abs(this.velocity) < .005) {
      this.position = this.target = wrap(this.target, this.ids.length);
      this.velocity = 0;
      this.token = undefined;
    } else this.token = requestAnimationFrame(this.tick);
    this.emit();
  };
  private emit() { this.changed({ position: this.position, moving: this.token !== undefined }); }
  private stop() {
    if (this.token !== undefined) cancelAnimationFrame(this.token);
    this.token = undefined;
    this.velocity = 0;
  }
  destroy() { this.stop(); }
}
