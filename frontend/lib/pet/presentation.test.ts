import { expect, it } from 'vitest';
import { initialPresentation, present } from './presentation';
import { initialSurface } from './surface';
const open = (state = docked()) => present(present(state, { type: 'enter' }), { type: 'open-delay' });
const docked = () => present(initialPresentation, { type: 'surface', surface: { ...initialSurface, mode: 'docked', revision: 1 } });
it('keeps a dropdown open while entering its body or holding keyboard focus', () => {
  let state = open();
  state = present(state, { type: 'leave' });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'enter' });
  expect(present(state, { type: 'close-delay' }).expanded).toBe(true);
  state = present(present(state, { type: 'leave' }), { type: 'focus' });
  expect(present(state, { type: 'close-delay' }).expanded).toBe(true);
  state = present(state, { type: 'blur' });
  expect(present(state, { type: 'close-delay' }).expanded).toBe(false);
});
it('resets the dropdown on detach and redock but retains desktop task folding', () => {
  let state = present(open(), { type: 'toggle-tasks' });
  state = present(state, { type: 'surface', surface: { ...initialSurface, revision: 2, dragging: true } });
  expect(state.expanded).toBe(false); expect(state.tasksCollapsed).toBe(true);
  expect(present(state, { type: 'enter' }).expanded).toBe(false);
  state = present(state, { type: 'surface', surface: { ...initialSurface, mode: 'docked', revision: 3 } });
  expect(state.expanded).toBe(false);
  expect(present(state, { type: 'surface', surface: { ...initialSurface, revision: 1 } })).toBe(state);
});
it('new tasks do not force a top panel open and Escape can collapse a hovered panel', () => {
  const state = present(open(), { type: 'collapse' });
  expect(present(state, { type: 'new-task' }).expanded).toBe(false);
});
it('keeps the same hover and keyboard presentation when notch clearance changes', () => {
  let state = open();
  state = present(state, { type: 'surface', surface: { ...state.surface, revision: 2, physicalNotch: true, topInset: 32 } });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'surface', surface: { ...state.surface, revision: 3, topInset: 0 } });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'leave' });
  expect(present(state, { type: 'close-delay' }).expanded).toBe(false);
  expect(present(state, { type: 'focus' }).expanded).toBe(true);
});

it('opens only after dwell, cancels on leave, and does not restart a cancelled dwell', () => {
  let state = present(docked(), { type: 'enter' });
  expect(state.expanded).toBe(false);
  expect(state.openPending).toBe(true);
  state = present(state, { type: 'leave' });
  expect(present(state, { type: 'open-delay' }).expanded).toBe(false);
  state = open(state);
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'collapse' });
  expect(present(state, { type: 'enter' }).openPending).toBe(false);
});

it('keeps an expanded panel open through press, horizontal dragging, and release', () => {
  let state = present(open(), { type: 'press' });
  state = present(present(state, { type: 'leave' }), { type: 'close-delay' });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'surface', surface: { ...state.surface, dragging: true } });
  state = present(present(state, { type: 'blur' }), { type: 'close-delay' });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'enter' });
  state = present(state, { type: 'surface', surface: { ...state.surface, dragging: false } });
  expect(state.expanded).toBe(true);
  expect(state.openPending).toBe(false);
});

it('cancels pending hover at press and never opens during a collapsed drag', () => {
  let state = present(present(docked(), { type: 'enter' }), { type: 'press' });
  expect(state.openPending).toBe(false);
  expect(present(state, { type: 'open-delay' }).expanded).toBe(false);
  state = present(state, { type: 'surface', surface: { ...state.surface, dragging: true } });
  state = present(present(state, { type: 'leave' }), { type: 'enter' });
  state = present(state, { type: 'focus' });
  expect(state.expanded).toBe(false);
  expect(state.openPending).toBe(false);
  state = present(state, { type: 'surface', surface: { ...state.surface, dragging: false } });
  expect(present(state, { type: 'open-delay' }).expanded).toBe(false);
  expect(present(state, { type: 'expand' }).expanded).toBe(true);
});
