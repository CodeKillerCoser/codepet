# Window Overflows Screen

## Symptom

The pet window extends past the edge of the monitor or cannot be fully seen.

## Evidence To Collect

- Platform and number of monitors.
- Monitor coordinates, including negative coordinates.
- Window outer position and outer size.
- Whether the issue happens on dock, drag, resize event, or cross-screen movement.

## Checks

1. Review `monitorForWindow()` in `frontend/PetApp.svelte`.
2. Review `clampWindowPositionToMonitor()`.
3. Confirm `ensureWindowFrameAndBounds()` is scheduled after move and resize events.
4. Confirm the selected monitor work area is correct.

## Validation After Fix

- Manual drag on every monitor edge.
- Manual cross-screen drag.
- Verify the task list remains visible after clamping.
