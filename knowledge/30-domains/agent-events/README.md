# Agent Events

This domain covers raw agent hook payloads, local ingestion, normalization, title enrichment, frontend merge, and activity filtering.

## Source Modules

- `src-tauri/hooks/code-pet-hook.mjs`: hook script installed into supported agent configs.
- `src-tauri/src/agent/hooks.rs`: writes managed hook entries.
- `src-tauri/src/activity/collector.rs`: receives hook payloads.
- `src-tauri/src/activity/events.rs`: normalizes payloads into `PetEvent`.
- `frontend/lib/activity.ts`: merges and filters user-visible activities.

## Tests

- `src-tauri/tests/hook_config_tests.rs`
- `src-tauri/tests/hook_script_tests.rs`
- `src-tauri/tests/event_normalizer_tests.rs`
- `frontend/lib/activity.test.ts`
