# Product Intent

Code Pet is a desktop pet companion for local AI coding tools. It makes background agent activity visible through a transparent floating pet window and keeps configuration in a normal main window.

## Product Goals

- Show task activity from Codex, Claude Code, Qoder, and Cursor without forcing the user to keep every agent window in view.
- Surface high-attention states such as permission requests, failures, and completed tasks.
- Provide lightweight actions from task cards when the backend has a reliable provider capability.
- Let the user personalize pet appearance, task bubble style, sounds, and pet window opacity.
- Keep all collector traffic local. The collector binds to `127.0.0.1`.

## Non-Goals

- Code Pet is not a replacement agent runtime.
- It should not claim active control capabilities for a provider until the provider path is verified.
- It should not turn README into the full knowledge base.

## Current Evidence

- `README.md` lists supported agents, local collector endpoint, settings data, and build/test commands.
- `src-tauri/src/activity/collector.rs` binds the collector to `127.0.0.1:47621`.
- `frontend/PetApp.svelte` renders the transparent pet window and task card actions.
- `frontend/App.svelte` renders the main configuration UI.

## Unknowns

- Long-term product naming between Hanging Metal and Code Pet is not normalized in code comments or docs.
