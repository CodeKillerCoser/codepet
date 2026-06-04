# Pet Window Known Risks

## Layout Regressions From Actions

Adding buttons to the task card footer can remove bottom spacing or create unnecessary scrollbars. Keep footer padding and card natural height in mind.

## Cross-Screen Dragging

The task list can appear compressed if window size or monitor bounds are stale during cross-screen drag. Keep size enforcement and bounds clamping together.

## Focus Regressions

Reply inputs can fail to focus if the editor is conditionally mounted and focus runs before the DOM update.

## Validation

- Inspect the pet window in one-card, multi-card, reply, approval, and collapsed states.
- Run relevant frontend tests before changing merge/action logic.
