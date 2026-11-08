import { expect, it } from 'vitest';
import { initialPresentation, present } from './presentation';
import { initialSurface } from './surface';
const docked = () => present(initialPresentation, { type: 'surface', surface: { ...initialSurface, mode: 'docked', revision: 1 } });
it('keeps a dropdown open while entering its body or holding keyboard focus', () => {
  let state = present(docked(), { type: 'enter' });
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
  let state = present(present(docked(), { type: 'enter' }), { type: 'toggle-tasks' });
  state = present(state, { type: 'surface', surface: { ...initialSurface, revision: 2, dragging: true } });
  expect(state.expanded).toBe(false); expect(state.tasksCollapsed).toBe(true);
  expect(present(state, { type: 'enter' }).expanded).toBe(false);
  state = present(state, { type: 'surface', surface: { ...initialSurface, mode: 'docked', revision: 3 } });
  expect(state.expanded).toBe(false);
  expect(present(state, { type: 'surface', surface: { ...initialSurface, revision: 1 } })).toBe(state);
});
it('new tasks do not force a top panel open and Escape can collapse a hovered panel', () => {
  const state = present(present(docked(), { type: 'enter' }), { type: 'collapse' });
  expect(present(state, { type: 'new-task' }).expanded).toBe(false);
});
it('native menu owns expansion across the gap, keyboard focus and delayed webview events', () => {
  let state = present(initialPresentation, { type: 'surface', surface: { ...initialSurface, revision: 1, mode: 'docked', menuBar: true, panelOpen: true } });
  for (const type of ['leave', 'blur', 'close-delay'] as const) state = present(state, { type });
  expect(state.expanded).toBe(true);
  state = present(state, { type: 'surface', surface: { ...state.surface, revision: 2, panelOpen: false } });
  expect(present(state, { type: 'focus' }).expanded).toBe(false);
  expect(present(state, { type: 'enter' }).expanded).toBe(false);
  state = present(state, { type: 'surface', surface: { ...state.surface, revision: 3, panelOpen: true } });
  state = present(state, { type: 'surface', surface: { ...state.surface, revision: 4, mode: 'floating', dragging: true, panelOpen: false } });
  expect(state.expanded).toBe(false);
});
