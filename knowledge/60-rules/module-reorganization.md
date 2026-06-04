# Module Reorganization

## Rule

Directory reorganization must preserve public module contracts, update documentation paths, and run compile-covering checks.

## Applies When

- Moving frontend entry files or shared libraries.
- Moving Rust modules under new domain directories.
- Renaming paths referenced by README, prompts, tests, or include macros.

## Counterexample

Moving `src-tauri/src/hooks.rs` into `src-tauri/src/agent/hooks.rs` without updating `include_str!` breaks compilation because the relative hook script path changes.

## Recommended Practice

Use `git mv` for tracked files, update entry points and config includes, keep compatibility re-exports when tests or public APIs depend on old module names, and search for stale paths after the move.

## Source

Code Pet frontend `src/` to `frontend/` rename and Tauri module reorganization.

## Verification

- `npx vitest run`
- `npm run build`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `git diff --check`
