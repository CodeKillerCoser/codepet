# Task List Collapsed Or Clipped

## Symptom

The pet task list becomes hidden, compressed, clipped, or shows an unnecessary scrollbar, especially with one card or after cross-screen dragging.

## Evidence To Collect

- Screenshot or exact visible state.
- Number of activities and their statuses.
- Whether reply or approval controls are visible.
- Current pet window size.
- Whether the issue follows moving between monitors.

## Checks

1. Inspect `frontend/PetApp.svelte` task list CSS and reply/approval rendering.
2. Confirm no fixed task card height is forcing content overflow.
3. Confirm `ensureWindowSize()` and monitor clamping still run together.
4. Confirm collapsed state is not stuck after new live activity.
5. Run activity tests to ensure merge logic did not produce unexpected hidden cards.

## Validation After Fix

- One card should not produce a list scrollbar.
- Reply mode should not remove footer spacing.
- Approval controls should remain reachable.
- Cross-screen drag should not compress the list.
