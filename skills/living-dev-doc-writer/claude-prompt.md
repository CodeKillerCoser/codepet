# Living Dev Doc Writer Prompt

Use this prompt in Claude Code when writing or reviewing living development documents.

You are writing a development document that must remain useful as the codebase evolves. Do not hard-code project facts unless you have read them from the repository, README, existing knowledge documents, source, tests, git history, logs, or user-provided evidence.

First classify the document:

- Development Spec: new feature, product change, behavior change, or planned architecture work.
- Bug Investigation: bug is still being analyzed and root cause is not proven.
- Bug Fix Record: bug is fixed or root cause and fix are known.
- Architecture Decision: durable technical choice.
- Development Rule: reusable constraint from repeated bugs or high-risk areas.

Choose intensity:

- Small: narrow UI tweak, small bug, single-module change. 300-700 words.
- Standard: normal feature or bug. 700-1400 words.
- Deep: cross-module, cross-platform, architecture, agent integration, data flow, windowing, or security-sensitive change. 1200-2200 words.

Common rules:

- Evidence before conclusions.
- Do not guess bug introduction history; write `Unconfirmed` if not verified.
- Every affected module must explain why it is relevant.
- Every risk must map to a test, check, or manual validation.
- Use `Unknowns` for missing facts.
- Do not write generic technology introductions.
- Do not propose implementation without validation.
- Bug fix records must include a regression guard.
- Repeated bugs or shared constraints should become Development Rules.

Use the matching template.

Development Spec:

```md
# <Feature or Change Name>

## Background
## Goals
## Non-Goals
## Current Understanding
## Implementation Path
## Affected Modules
## Risks
## Test Plan
## Knowledge Evolution
## Unknowns
```

Bug Investigation:

```md
# <Bug Title>

## Symptom
## Reproduction Path
## Evidence
## Initial Impact Scope
## Hypotheses
## Ruled Out
## Next Debug Steps
## Unknowns
```

Bug Fix Record:

```md
# <Bug Title>

## Symptom
## Evidence
## Root Cause
## Introduction History
## Fix
## Affected Modules
## Validation
## Regression Guard
## Rule Candidate
## Unknowns
```

Architecture Decision:

```md
# <Decision Title>

## Background
## Decision
## Alternatives Considered
## Rationale
## Impact
## Follow-Up
```

Development Rule:

```md
# <Rule Title>

## Rule
## Applies When
## Counterexample
## Recommended Practice
## Source
## Verification
```

Before finalizing, check that the document is concrete, scoped, evidence-backed, validation-aware, and clear about whether knowledge should evolve.
