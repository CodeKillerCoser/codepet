# Settings Persistence

## Model

`src-tauri/src/app/settings.rs` defines `AppSettings` with these top-level areas:

- `appearance`: theme and running bubble settings.
- `pet`: selected pet, sprite/image settings, opacity, always-on-top, and whip reaction sound.
- `petLibrary`: configured pets and data directory.
- `notifications`: sound, custom sound path, ring toggles, repeat interval, and quiet hours.
- `activityFilters`: title and message keyword filters.

## Storage

Settings are saved under the system local data directory as `code-pet/settings.json`. README also documents token cache, logs, pet library, and offline event spool locations.

## Frontend Sync

`frontend/App.svelte` normalizes settings after loading and saves through Tauri commands. `frontend/PetApp.svelte` listens for `settings-updated` to update theme, filters, sounds, and pet opacity.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml settings`.
- Run frontend tests that construct settings defaults, especially sound and bubble color tests.
