import { initialSurface, type SurfaceView } from './surface';

export interface Presentation {
  surface: SurfaceView;
  expanded: boolean;
  hovered: boolean;
  focused: boolean;
  pointerHeld: boolean;
  openPending: boolean;
  tasksCollapsed: boolean;
}
export const initialPresentation: Presentation = {
  surface: initialSurface, expanded: false, hovered: false, focused: false,
  pointerHeld: false, openPending: false, tasksCollapsed: false,
};
export type PresentationEvent =
  | { type: 'surface'; surface: SurfaceView }
  | { type: 'enter' | 'leave' | 'focus' | 'blur' | 'press' | 'release' | 'open-delay' | 'close-delay' | 'expand' | 'collapse' | 'toggle-tasks' | 'new-task' };

// Native surface owns docking/drag transactions; this reducer owns presentation only.
export function present(state: Presentation, event: PresentationEvent): Presentation {
  const locked = state.pointerHeld || state.surface.dragging;
  switch (event.type) {
    case 'surface': {
      if (event.surface.revision < state.surface.revision) return state;
      if (event.surface.mode !== state.surface.mode) {
        return { ...state, surface: event.surface, expanded: false, hovered: false, focused: false, pointerHeld: false, openPending: false };
      }
      const dragChanged = event.surface.dragging !== state.surface.dragging;
      // Moving along the top must preserve the layout grabbed by the pointer.
      return { ...state, surface: event.surface, ...(dragChanged ? { pointerHeld: false, openPending: false } : {}) };
    }
    case 'enter': return { ...state, hovered: true, openPending: state.openPending || (!state.hovered && !state.expanded && !locked && state.surface.mode === 'docked') };
    case 'leave': return { ...state, hovered: false, openPending: false };
    case 'press': return { ...state, pointerHeld: true, openPending: false };
    case 'release': return { ...state, pointerHeld: false };
    case 'open-delay': return { ...state, openPending: false, expanded: state.expanded || (state.openPending && state.hovered && !locked && state.surface.mode === 'docked') };
    case 'expand': return locked || state.surface.mode !== 'docked' ? state : { ...state, expanded: true, openPending: false };
    case 'focus': return { ...state, focused: true, ...(locked ? {} : { expanded: state.surface.mode === 'docked', openPending: false }) };
    case 'blur': return { ...state, focused: false };
    case 'close-delay': return locked || state.hovered || state.focused ? state : { ...state, expanded: false };
    case 'collapse': return locked ? state : { ...state, expanded: false, openPending: false };
    case 'toggle-tasks': return { ...state, tasksCollapsed: !state.tasksCollapsed };
    case 'new-task': return { ...state, tasksCollapsed: false };
  }
}
