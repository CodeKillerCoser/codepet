# Project Knowledge

This directory is the entry point for durable project facts. It explains what Code Pet is and where future agents should start before changing code.

## Read First

- `product-intent.md`: product purpose, audience, and non-goals.
- `../10-architecture/README.md`: runtime shape and module boundaries.
- `../30-domains/README.md`: feature domains with deeper implementation facts.
- `../40-runbooks/README.md`: repeatable debugging procedures.
- `../60-rules/README.md`: reusable development constraints.

## Evidence Sources

- `README.md` for user-facing capabilities and local development commands.
- `frontend/` for Svelte UI, task card behavior, settings UI, theme tokens, and sound logic.
- `src-tauri/src/` for Rust backend, collector, settings persistence, agent hooks, and platform behavior.
- `src-tauri/tests/` and `frontend/**/*.test.ts` for protected behavior.

## Maintenance Rule

Keep this tree semantic. Do not add `map.yaml` or a similar central index. A future agent should understand the knowledge shape by reading directory and document names.
