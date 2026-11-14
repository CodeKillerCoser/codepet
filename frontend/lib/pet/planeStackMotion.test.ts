import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PlaneStackMotion, planeStackCards, type PlaneStackFrame } from './planeStackMotion';
let motion: PlaneStackMotion, frame: PlaneStackFrame;
beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now());
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => setTimeout(() => fn(performance.now()), 16));
  vi.stubGlobal('cancelAnimationFrame', clearTimeout);
  motion = new PlaneStackMotion(value => { frame = value; });
  motion.sync(['a', 'b', 'c', 'd', 'e'], true, false);
});
afterEach(() => { motion.destroy(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('keeps bounded, unique planes through both loop seams and two-card swaps', () => {
  for (const count of [1, 2, 3, 18, 1000]) {
    const items = Array.from({ length: count }, (_, i) => i);
    for (const position of [-1, -.5, -.001, 0, .499, .5, .501, 1, count - .001, count]) {
      const cards = planeStackCards(items, position);
      expect(new Set(cards.map(card => card.item)).size).toBe(cards.length);
      expect(cards.length).toBeLessThanOrEqual(3);
      expect(cards.filter(card => card.front)).toHaveLength(1);
      for (const card of cards) {
        expect(card.scale).toBeGreaterThanOrEqual(600 / 636);
        expect(card.scale).toBeLessThanOrEqual(1);
        expect(card.opacity).toBeGreaterThanOrEqual(0);
      }
    }
    expect(planeStackCards(items, count)).toEqual(planeStackCards(items, 0));
  }
});
it('continuously brings the next plane forward and retires the current one', () => {
  const frames = Array.from({ length: 61 }, (_, i) => planeStackCards(['a','b','c','d'], i / 60));
  for (let i = 1; i < frames.length; i++) for (const card of frames[i]) {
    const previous = frames[i - 1].find(item => item.item === card.item);
    if (!previous) { expect(card.opacity).toBeLessThan(.05); continue; }
    expect(Math.abs(card.y - previous.y)).toBeLessThan(2);
    expect(Math.abs(card.scale - previous.scale)).toBeLessThan(.01);
    expect(Math.abs(card.ink - previous.ink)).toBeLessThan(.05);
  }
  expect(frames[30].find(card => card.item === 'a')!.y).toBeLessThan(frames[30].find(card => card.item === 'b')!.y);
  expect(frames[30].find(card => card.item === 'b')!.y).toBeGreaterThan(0);
  expect(frames.at(-1)![0].item).toBe('b');
});
it('retargets a moving spring without jumping and settles after a reversal', async () => {
  motion.step(1);
  await vi.advanceTimersByTimeAsync(96);
  expect(frame.position).toBeGreaterThan(0); expect(frame.position).toBeLessThan(1);
  const position = frame.position;
  motion.step(-1);
  expect(frame.position).toBe(position);
  await vi.advanceTimersByTimeAsync(1200);
  expect(frame).toEqual({ position: 0, moving: false });
});
it('preserves selected identity on data updates and stops when expanded or removed', async () => {
  motion.step(-1); await vi.advanceTimersByTimeAsync(1200);
  expect(frame.position).toBe(4);
  motion.sync(['new','a','b','c','d','e'], true, false);
  expect(frame.position).toBe(5);
  motion.step(1); await vi.advanceTimersByTimeAsync(96);
  motion.sync(['new','a','b','c','d','e'], false, false);
  const stopped = frame;
  await vi.advanceTimersByTimeAsync(1200); motion.step(1);
  expect(frame).toEqual(stopped); expect(frame.moving).toBe(false);
  motion.sync(['b','c'], true, false);
  expect(frame.position).toBe(0);
});
it('respects reduced motion and cancels all frames on destruction', async () => {
  motion.sync(['a','b'], true, true); motion.step(1);
  expect(frame).toEqual({ position: 1, moving: false });
  expect(vi.getTimerCount()).toBe(0);
  motion.sync(['a','b'], true, false); motion.step(1);
  motion.destroy(); expect(vi.getTimerCount()).toBe(0);
});

it('returns to the original compact stack at every resting slot', () => {
  for (const count of [2, 3, 7]) for (const position of [-1, 0, 1, count]) {
    const cards = planeStackCards(Array.from({ length: count }, (_, i) => i), position);
    expect(cards[0].front).toBe(true);
    expect(cards[0].y).toBe(0);
    expect(cards[0].scale).toBe(1);
    const layers = [...cards].sort((a, b) => b.scale - a.scale);
    layers.forEach((card, depth) => {
      expect(card.scale).toBeCloseTo(600 / (600 + 18 * depth));
      expect(card.y).toBeCloseTo(11 * depth * card.scale);
      expect(card.layer).toBe(2000 - depth * 750);
      if (depth) expect(card.ink).toBe(0);
    });
  }
});
