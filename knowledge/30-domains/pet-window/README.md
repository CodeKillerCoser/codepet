# Pet Window Domain

This domain covers the floating window, task card layout, window sizing, monitor bounds, and reply editor.

## Source Modules

- `frontend/PetApp.svelte`: window UI, sizing, docking, drag, reply, approval, sound repeat.
- `frontend/lib/petHitTest.ts`: hit-test rectangle support.
- `src-tauri/src/platform/macos_window.rs`: macOS overlay configuration.

## Tests

- `frontend/PetApp.test.ts`
- `frontend/lib/petHitTest.test.ts`
- `src-tauri/tests/macos_window_tests.rs`
