# Activity Merge

## Responsibility

`frontend/lib/activity.ts` turns recent event history into the compact list shown by the pet window.

## Current Rules

- The key is `provider:sessionId`, falling back to `provider:cwd`, then `provider:global`.
- Internal Codex background prompts are filtered in code.
- User-defined title and message filters are applied through `ActivityFilterSettings`.
- Hidden activity keys are retained across incremental batches.
- Stale thinking/running activities are removed after 30 minutes.
- Completed and failed activities use `endedAt` or `createdAt` for footer time.

## Why This Matters

Most visible task card regressions come from grouping, hidden-key persistence, or terminal event handling. Treat this module as a shared behavior surface.

## Validation

- Run the full `frontend/lib/activity.test.ts` suite.
- Manually inspect the pet window after changing merge behavior.
