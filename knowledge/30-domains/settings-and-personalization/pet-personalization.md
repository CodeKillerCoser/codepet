# Pet Personalization

## Current Behavior

Users can select default palette pets, import image pets, adjust image pixel size, choose a pet data directory, and delete non-default pets.

`settings.pet.opacity` controls the pet window overall opacity and is exposed in the personalization UI.

## Implementation

- `src-tauri/src/pet/library.rs` handles library listing, import, selection, deletion, data directory changes, image pixelation, and built-in Codex atlas pet discovery.
- `src-tauri/src/pet/subject_cutout.rs` handles cutout requests.
- `frontend/App.svelte` owns the personalization controls.
- `frontend/PetApp.svelte` applies pet opacity and renders the selected avatar.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml pet_library_tests subject_cutout_tests`.
- Manually verify imported image, default pet, and opacity changes in the pet window.
