# Pet Window Experience

## Behavior

The pet window is a transparent floating surface for daily activity monitoring. It shows the current pet avatar, task cards, action buttons, notification messages, and whip interaction.

## Current Facts

- `frontend/PetApp.svelte` is loaded from `pet.html`.
- The pet window loads settings, listens to `pet-event`, polls `recentEvents()`, and listens for `settings-updated`.
- New live activities can expand a collapsed task list.
- Pet opacity is controlled by `settings.pet.opacity` and applied as `--pet-window-opacity`.
- The window is programmatically sized and constrained to monitor bounds.

## Product Constraints

- The pet should stay useful while the user reads other content, so opacity and task list density matter.
- Transparent area mouse behavior has been simplified by using a preset window size.
- Reply or approval UI must not appear during unsupported task phases.

## Validation

- Check `frontend/PetApp.svelte` for layout and action state changes.
- Run `npx vitest run frontend/lib/activity.test.ts frontend/lib/sound.test.ts` after changing card behavior or sound behavior.
