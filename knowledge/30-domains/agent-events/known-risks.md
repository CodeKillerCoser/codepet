# Agent Events Known Risks

## Background Agent Noise

Codex can emit internal background tasks such as memory summaries, personalized suggestions, or task title generation. Some known patterns are hard-coded in `frontend/lib/activity.ts`, and user-defined filters live in settings.

Risk: relying only on hard-coded filters cannot cover future internal prompts.

Validation: add frontend tests for title and message filters when changing filtering.

## Orphan Terminal Events

Terminal `done` events without stable identity can create misleading cards. Current merge logic drops orphan completed sessions unless it can match a visible active provider card.

Validation: keep tests covering orphan completed sessions in `frontend/lib/activity.test.ts`.

## Provider Field Drift

Agents can change payload fields. Normalization should tolerate aliases and preserve raw data internally while sending sanitized frontend events.

Validation: add event normalizer fixtures before changing extraction rules.
