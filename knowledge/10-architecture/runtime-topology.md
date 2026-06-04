# Runtime Topology

## Components

- Main window: `frontend/App.svelte`, loaded from `index.html`.
- Pet overlay window: `frontend/PetApp.svelte`, loaded from `pet.html`.
- Tauri backend: `src-tauri/src/lib.rs` registers commands, tray behavior, plugins, startup work, and windows.
- Local collector: `src-tauri/src/activity/collector.rs` exposes HTTP routes on `127.0.0.1:47621`.
- Hook script: `src-tauri/hooks/code-pet-hook.mjs` is installed into the local app data directory by `src-tauri/src/agent/hooks.rs`.

## Flow

1. The user enables an Agent in the main window.
2. Rust writes managed hook entries into that Agent config.
3. The Agent invokes `code-pet-hook.mjs`.
4. The script posts a payload to `/hook`.
5. Rust normalizes and stores a `PetEvent`.
6. Rust emits `pet-event` to the pet window.
7. The pet window merges events into task cards and plays sounds when settings allow.

## Validation

- Frontend behavior is covered by `frontend/lib/activity.test.ts`, `frontend/lib/sound.test.ts`, and component tests.
- Backend collector and hook behavior is covered by tests under `src-tauri/tests/`.
