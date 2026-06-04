# Theme Migration Rules

## Rule

New production UI styles should not introduce raw hex, rgb, hsl, or named colors outside the theme library unless the value is user-authored content or an external asset requirement.

## Recommended Practice

- Add or reuse a semantic token in `frontend/lib/theme/tokens.css`.
- Use component aliases when updating a specific surface.
- Keep TypeScript defaults in `frontend/lib/theme/defaults.ts`.
- Keep Rust persisted defaults in `src-tauri/src/pet/theme_defaults.rs` aligned when a default crosses the backend boundary.

## Validation

- Use `rg` to search for raw color literals in `frontend/`.
- Run frontend tests after token migration.
