# Theme Library

Code Pet uses Radix Colors as primitive palettes and exposes project-owned CSS tokens.

Use these layers in order:

1. Primitive palettes from `@radix-ui/colors` stay inside `tokens.css`.
2. Semantic tokens use `--color-*`, `--font-*`, `--line-height-*`, and `--letter-spacing-*`.
3. Component aliases keep existing surfaces stable through `--app-*` and `--pet-*`.
4. Asset tokens use `--asset-*` for pixel pet and whip artwork.

Production UI styles should not introduce raw hex or rgb values outside this folder. User-editable colors such as running bubble gradients and pet sprite colors remain persisted data, but their defaults come from `defaults.ts`.
