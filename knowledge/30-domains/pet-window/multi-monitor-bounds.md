# Multi Monitor Bounds

## Current Implementation

`frontend/PetApp.svelte` keeps the pet window inside the selected monitor work area.

Selection rules:

1. Use the monitor with the largest intersection with the window.
2. If no monitor intersects, choose the monitor nearest the window center.
3. If monitor lookup fails, fall back to the primary monitor.

The final position is clamped between the monitor work area top-left and the maximum position that still keeps the whole window inside the work area.

## Risks

- Multi-monitor setups can use negative coordinates.
- macOS cross-screen dragging can produce brief stale geometry.
- Resizing and moving can trigger each other, so `ensureWindowFrameAndBounds()` guards against concurrent execution.

## Validation

- Drag across monitors on macOS and Windows.
- Verify right and bottom edges remain inside the visible work area.
- Verify task list does not become clipped after a cross-screen drag.
