# Settings Model

## Current Model

`AppSettings` in `src-tauri/src/app/settings.rs` has five top-level areas:

- `appearance`
- `pet`
- `petLibrary`
- `notifications`
- `activityFilters`

All sections use serde defaults so old settings files can load when new fields are added.

## Frontend Normalization

`frontend/App.svelte` normalizes loaded settings before use:

- running bubble defaults and numeric bounds.
- image pixel size.
- pet opacity.
- whip reaction sound defaults.
- activity filter keyword trimming and deduplication.

## Risk

Adding a field without a Rust default or frontend normalization can break old users or produce `undefined` UI state.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml settings_tests`.
- Add or update frontend tests when default values affect UI helpers.
