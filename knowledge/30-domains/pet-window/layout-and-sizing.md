# Layout And Sizing

## Current Sizing Model

The pet window uses a preset logical width and height in `frontend/PetApp.svelte`. User resize is disabled, but the app still calls `setSize()` so the window frame stays predictable.

Task card content is not supposed to depend on fixed card heights. Reply editor and footer spacing need to fit naturally inside the card.

## Known UI Constraints

- A single task card should not cause unnecessary list scrollbars.
- Footer spacing must remain visible when action buttons are present.
- Reply mode can expand over the pet window area, but should keep controls reachable.
- Text must not overlap card footer or buttons.

## Validation

- Manually inspect one card, several cards, reply mode, approval mode, and collapsed task list.
- Run `npx vitest run frontend/PetApp.test.ts frontend/lib/activity.test.ts` after layout changes.
