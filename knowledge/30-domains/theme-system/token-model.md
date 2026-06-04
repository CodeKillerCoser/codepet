# Token Model

## Layers

The frontend theme library uses these layers:

1. Radix primitive palettes inside `frontend/lib/theme/tokens.css`.
2. Semantic tokens such as `--color-*`, `--font-*`, `--line-height-*`, and `--letter-spacing-*`.
3. Component aliases such as `--app-*` and `--pet-*`.
4. Asset tokens such as `--asset-*`.

## Theme Classes

`frontend/lib/theme/defaults.ts` exposes `themeClassNames()`, which returns the class set for light or dark mode. `frontend/App.svelte` and `frontend/PetApp.svelte` apply those classes based on settings and system preference.

## Validation

- Run `npx vitest run frontend/styles.test.ts` and theme-related tests when changing tokens.
- Search for raw color literals outside `frontend/lib/theme/` before completing UI color work.
