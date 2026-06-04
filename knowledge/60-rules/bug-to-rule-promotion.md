# Bug To Rule Promotion

## Rule

When a bug reveals a reusable constraint, update the relevant runbook or add a rule. Do not leave the lesson only in chat.

## Applies When

- The same class of bug has happened more than once.
- The fix touches shared UI state, provider capabilities, window behavior, or settings persistence.
- A review finds a regression risk that future agents are likely to repeat.

## Counterexample

A reply button is moved based on local UI convenience, but the provider capability boundary is not documented. A later agent exposes the same button during running tasks and breaks the workflow again.

## Recommended Practice

Write a short rule with source, counterexample, and validation. Link to domain docs or runbooks by path.

## Source

Project discussion about keeping living documents aligned with every change and bug fix.

## Verification

After bug fixes, check whether `knowledge/40-runbooks/`, `knowledge/50-decisions/`, or `knowledge/60-rules/` needs an update.
