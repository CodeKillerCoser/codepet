# Theme Library

Code Pet uses Radix Colors as primitive palettes and exposes project-owned CSS tokens and multi-theme presets.

Use these layers in order:

1. Primitive palettes from `@radix-ui/colors` stay inside `tokens.css`.
2. Semantic typography tokens (`--font-title`, `--font-section`, `--font-subheading`, `--font-body`, `--font-caption`) and strict 3-weight tokens (`400 / 500 / 600`) live in `tokens.css`.
3. Main-window semantic roles and 4 presets (`warm-stone`, `cool-slate`, `forest-sage`, `caramel-earth`), each with light and dark pairs, live in `main-window.css`.
4. Component aliases keep existing surfaces stable through `--app-*`, `--wb-*`, and `--pet-*`.
5. Asset tokens use `--asset-*` for whip artwork.

Production UI styles must not introduce raw hex or rgb values outside this folder. User-editable colors such as running bubble gradients remain persisted data, with defaults in `defaults.ts`.
