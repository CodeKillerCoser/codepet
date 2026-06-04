# Theme System

This domain covers the project theme library and migration away from hard-coded UI colors.

## Source Modules

- `frontend/lib/theme/tokens.css`: primitive, semantic, component, and asset CSS tokens.
- `frontend/lib/theme/defaults.ts`: TypeScript defaults and theme class names.
- `frontend/lib/theme/index.ts`: public theme exports.
- `frontend/lib/theme/README.md`: theme library rules.
- `src-tauri/src/pet/theme_defaults.rs`: Rust defaults matching persisted settings needs.

## Principle

Production UI styles should use theme tokens. User-editable colors may remain persisted data, but defaults should come from the theme library.
