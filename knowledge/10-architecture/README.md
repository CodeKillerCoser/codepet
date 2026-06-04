# Architecture

Code Pet is a Tauri 2 application with a Svelte frontend and Rust backend. The frontend owns UI state and interaction ergonomics. The backend owns local integration points, persistence, event ingestion, and platform APIs.

## Main Areas

- `runtime-topology.md`: windows, collector, IPC, tray, and local runtime flow.
- `frontend-backend-boundary.md`: what belongs in Svelte versus Rust.
- `event-pipeline.md`: hook payload to pet task card.
- `settings-persistence.md`: settings model and storage flow.
- `window-system.md`: pet window sizing, docking, and monitor bounds.
- `agent-control.md`: hooks, activation, reply, and approval boundaries.
- `cross-platform-boundaries.md`: macOS, Windows, and unsupported paths.

## Module Evidence

- `frontend/` contains Svelte entry points and frontend libraries.
- `src-tauri/src/activity/` contains collector, events, title resolution, and token usage.
- `src-tauri/src/agent/` contains agent registry, hook management, provider control, and provider-specific helpers.
- `src-tauri/src/app/` contains settings, shared state, logs, autostart, and CLI helpers.
- `src-tauri/src/pet/` contains pet library, image processing, subject cutout, and theme defaults.
- `src-tauri/src/platform/` contains platform-specific window code.
