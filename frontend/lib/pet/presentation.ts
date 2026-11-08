import { initialSurface, type SurfaceView } from './surface';

export interface Presentation {
  surface: SurfaceView;
  expanded: boolean;
  hovered: boolean;
  focused: boolean;
  tasksCollapsed: boolean;
}
export const initialPresentation: Presentation = {
  surface: initialSurface, expanded: false, hovered: false, focused: false, tasksCollapsed: false,
};
export type PresentationEvent =
  | { type: 'surface'; surface: SurfaceView }
  | { type: 'enter' | 'leave' | 'focus' | 'blur' | 'close-delay' | 'collapse' | 'toggle-tasks' | 'new-task' };

// Native surface owns docking/drag transactions; this reducer owns presentation only.
export function present(state: Presentation, event: PresentationEvent): Presentation {
  switch (event.type) {
    case 'surface': {
      if (event.surface.revision < state.surface.revision) return state;
      const changed = event.surface.mode !== state.surface.mode || event.surface.dragging !== state.surface.dragging;
      return { ...state, surface: event.surface, ...(changed ? { expanded: false, hovered: false, focused: false } : {}) };
    }
    case 'enter': return { ...state, hovered: true, expanded: state.surface.mode === 'docked' && !state.surface.dragging };
    case 'focus': return { ...state, focused: true, expanded: state.surface.mode === 'docked' && !state.surface.dragging };
    case 'leave': return { ...state, hovered: false };
    case 'blur': return { ...state, focused: false };
    case 'close-delay': return state.hovered || state.focused ? state : { ...state, expanded: false };
    case 'collapse': return { ...state, expanded: false };
    case 'toggle-tasks': return { ...state, tasksCollapsed: !state.tasksCollapsed };
    case 'new-task': return { ...state, tasksCollapsed: false };
  }
}
