# Agent Collaboration Guide

This file is the entry protocol for AI agents working in Code Pet / Hanging Metal. It should stay short. Project facts belong in `knowledge/`. Repo-local reusable skills belong in `.agents/skills/` so Codex can load them from the workspace.

## Before Editing

- Read the relevant source and nearby tests before proposing or changing code.
- Search `knowledge/` by semantic directory and document title. Do not rely only on chat history.
- Check `git status --short` and preserve unrelated user changes.
- Identify affected modules and older behavior that could regress.

## During Editing

- Keep changes scoped to the requested behavior.
- Prefer existing module boundaries, APIs, tokens, and test style.
- Do not add a central knowledge map. The `knowledge/` file tree and document titles are the index.
- For frontend UI changes, verify layout, focus behavior, action visibility, and responsive sizing when relevant.
- For Tauri/Rust changes, verify module paths and public re-exports used by tests.

## After Editing

- Run the smallest meaningful automated checks, then broaden when the change crosses module or platform boundaries.
- Record commands that were run and any checks that could not be run.
- If a bug fix reveals a reusable constraint, update or create a rule under `knowledge/60-rules/`.
- If a fix creates a repeatable diagnostic path, update or create a runbook under `knowledge/40-runbooks/`.
- Do not leave important conclusions only in chat.

## Documentation

- Use `.agents/skills/living-dev-doc-writer/` when writing or reviewing living development documents.
- Keep skill instructions in `SKILL.md`; keep optional reusable prompts or reference material under that skill's `references/` directory.
- Evidence comes before conclusions in bug documents.
- Every affected module needs a reason, and every risk needs a validation path.
