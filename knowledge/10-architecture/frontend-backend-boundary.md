# Frontend Backend Boundary

## Frontend Responsibilities

- Render the main window and pet overlay.
- Merge recent events into user-facing task cards.
- Decide card action visibility from provider capabilities.
- Manage focus, reply editor state, notification repeat timers, and layout.
- Apply theme classes and CSS tokens.

Relevant modules: `frontend/App.svelte`, `frontend/PetApp.svelte`, `frontend/lib/activity.ts`, `frontend/lib/agentInteractions.ts`, `frontend/lib/theme/`.

## Backend Responsibilities

- Install and remove managed hook entries.
- Run the local collector and normalize raw hook payloads.
- Persist settings and pet library data.
- Resolve approval waits, send verified replies, and activate supported targets.
- Provide platform-specific window setup and local logging.

Relevant modules: `src-tauri/src/agent/`, `src-tauri/src/activity/`, `src-tauri/src/app/`, `src-tauri/src/platform/`.

## Rule

Action buttons in the frontend must be driven by backend-capability-compatible logic. Do not expose a button only because a raw hook event contains enough display data.
