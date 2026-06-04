# Notification Settings

## Current Settings

`NotificationSettings` includes:

- selected sound.
- custom sound path.
- ring toggles for permission, failure, and done.
- repeat seconds for waiting approval.
- quiet hours.

Whip reaction sound is stored under `PetSettings` because it belongs to pet interaction rather than task notifications.

## Runtime Behavior

`frontend/PetApp.svelte` calls `handleRing()` when new events arrive. It uses `frontend/lib/sound.ts` for ring decisions, quiet hours, sound playback, and repeat behavior.

## Validation

- Run `npx vitest run frontend/lib/sound.test.ts`.
- Manually verify repeat approval sound stops when approval is resolved or the activity disappears.
