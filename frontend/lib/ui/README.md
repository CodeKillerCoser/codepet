# Project UI components

Adapted from shadcn-svelte (MIT), pinned to commit
`3c4152d7d3904e76f6f756ec8ffca7c9ec72f977`:
https://github.com/huntabyte/shadcn-svelte/tree/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/docs/src/lib/registry/ui

Keep upstream bindable refs, values, snippets, native attributes and Bits UI behavior.
Native Button/Input/Textarea refs use `$bindable()` without a fallback so a parent
can bind an initially undefined DOM reference. `button.test.ts` covers initial
undefined/null values and repeated conditional mounting for all three controls.
Local changes replace stock class presets with existing Code Pet theme tokens;
Dialog portals render in place by default to inherit main-window theme variables.
Title/Description retain native h3/p tags through Bits UI child snippets. An
overlayClass can preserve a feature's existing backdrop. Default visual variants
inherit the project's current control styles; this directory does not ship the
upstream stock skin. Add explicit token-backed styling before using new variants.
Icons continue to come from the existing @lucide/svelte dependency.
Tailwind utilities are scoped to this source directory and Preflight is not loaded.
No Radix palette or icon dependency changes are required.

Before updating: compare upstream contracts, run the frontend suite and production
build, then check dialog focus/Escape, tab navigation, settings persistence and
before/after screenshots with `npm run test:ui`.
Use `CODEPET_QA_ENGINE=webkit npm run test:ui` for the WebKit regression as well.
