# Settings And Personalization

This domain covers persisted settings, pet library data, notification settings, and personalization UI.

## Source Modules

- `src-tauri/src/app/settings.rs`: persisted settings structs, defaults, load/save.
- `src-tauri/src/pet/library.rs`: pet library and image import.
- `src-tauri/src/pet/subject_cutout.rs`: subject cutout.
- `frontend/App.svelte`: settings UI and normalization before save.
- `frontend/PetApp.svelte`: consumes live settings updates.

## Tests

- `src-tauri/tests/settings_tests.rs`
- `src-tauri/tests/pet_library_tests.rs`
- `src-tauri/tests/subject_cutout_tests.rs`
- frontend tests for sound, bubble colors, and theme-related helpers.
