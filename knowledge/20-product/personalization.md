# Personalization

## Supported Areas

- Theme mode: system, light, dark.
- Pet appearance: default pixel palette, imported images, Codex atlas pets, pixel size, and pet library selection.
- Pet window opacity: range controlled by the personalization UI.
- Running bubble: background breathing, border marquee, colors, border width, and animation speed.
- Sounds: notification sound, custom notification sound, quiet hours, and whip reaction sound.

## Implementation

- Settings model: `src-tauri/src/app/settings.rs`.
- Main UI: `frontend/App.svelte`.
- Theme tokens: `frontend/lib/theme/`.
- Sound behavior: `frontend/lib/sound.ts`.
- Pet library and image processing: `src-tauri/src/pet/library.rs` and `src-tauri/src/pet/subject_cutout.rs`.

## Validation

- Run `npx vitest run frontend/lib/bubbleColorSettings.test.ts frontend/lib/sound.test.ts`.
- Run Rust pet/settings tests when changing persisted defaults or pet library behavior.
