---
name: living-dev-doc-writer
description: Use when writing or reviewing living development documents for a codebase, including feature specs, bug investigations, bug fix records, architecture decisions, and development rules. The skill standardizes evidence, scope, risk, validation, and knowledge-evolution sections without embedding project-specific facts.
metadata:
  short-description: Write living development docs
---

# Living Dev Doc Writer

Use this skill when asked to write, normalize, or review a development document that should stay useful as the codebase evolves.

This skill describes how to write. It must not hard-code project facts. Read project facts from the repository, README, existing knowledge documents, source, tests, git history, logs, or user-provided evidence.

Codex loads this as a repo-local skill from `.agents/skills/living-dev-doc-writer/`. Claude Code can reuse the compact prompt in `references/claude-prompt.md`.

## Workflow

1. Identify the document type:
   - Feature or product change: use Development Spec.
   - Bug still under investigation: use Bug Investigation.
   - Bug already fixed or fix is known: use Bug Fix Record.
   - Important technical choice: use Architecture Decision.
   - Reusable constraint from repeated bugs or high-risk areas: use Development Rule.
2. Choose intensity:
   - Small: narrow UI tweak, small bug, single-module change. Target 300-700 words.
   - Standard: normal feature or bug. Target 700-1400 words.
   - Deep: cross-module, cross-platform, architecture, agent integration, data flow, windowing, or security-sensitive change. Target 1200-2200 words.
3. Gather evidence before writing conclusions.
4. Keep the document action-oriented. Avoid generic technology introductions.
5. Use `Unknowns` when information is missing. Do not invent evidence, history, or decisions.

## Common Rules

- Every affected module must explain why it is relevant.
- Every risk must have a matching test, check, or manual validation path.
- Bug documents must be evidence-first; do not start from an unsupported root cause.
- Bug introduction history must not be guessed. If not verified with git history or comparable evidence, write `Unconfirmed`.
- A document that proposes an implementation must include validation.
- A bug fix record must include a regression guard.
- Repeated bugs, public-module regressions, cross-platform constraints, or recurring UI/state failures should be promoted into a Development Rule.
- Keep examples and wording concrete. Prefer file paths, command names, observed behavior, and test names over broad prose.

## Development Spec

Use for new features, product changes, behavior changes, or planned architecture work.

Required sections:

```md
# <Feature or Change Name>

## Background
Why this change is needed. Mention the user pain, product gap, or technical pressure.

## Goals
- Concrete outcomes this work must achieve.

## Non-Goals
- Explicitly excluded work to prevent scope expansion.

## Current Understanding
Current product behavior, code shape, constraints, and any relevant existing knowledge.

## Implementation Path
Recommended approach, data flow, state flow, edge cases, and boundaries.

## Affected Modules
- `<path or module>`: why it is involved.

## Risks
- Risk: validation or mitigation.

## Test Plan
- Automated tests to add or run.
- Manual checks when automation is insufficient.

## Knowledge Evolution
State whether this should update feature docs, runbooks, decisions, rules, or bug records.

## Unknowns
- Open questions or facts not yet verified.
```

Small specs may omit `Non-Goals` only when the scope is obvious. Deep specs must keep all sections.

## Bug Investigation

Use while the bug is still being analyzed and the root cause is not proven.

Required sections:

```md
# <Bug Title>

## Symptom
What the user sees. Include platform, timing, frequency, and visible failure mode when known.

## Reproduction Path
Steps, environment, and whether reproduction is stable, intermittent, or unavailable.

## Evidence
- Logs, screenshots, failing tests, code references, user reports, timestamps, or observed state.

## Initial Impact Scope
Likely affected features, modules, platforms, and user workflows.

## Hypotheses
- Hypothesis: supporting evidence and confidence.

## Ruled Out
- Direction checked: why it is unlikely or disproven.

## Next Debug Steps
- The next concrete checks to run.

## Unknowns
- Missing facts that block stronger conclusions.
```

Do not include a final root cause unless it is proven. If evidence changes, update the document instead of appending disconnected notes.

## Bug Fix Record

Use after the root cause and fix are known.

Required sections:

```md
# <Bug Title>

## Symptom
External behavior that failed.

## Evidence
- The observations, logs, tests, code references, or reproduction results that support the diagnosis.

## Root Cause
The mechanism that caused the bug. Tie it back to evidence.

## Introduction History
Commit, change, release, or code evolution that introduced the bug.
Write `Unconfirmed` if not verified.

## Fix
What changed and why this fix is appropriate.

## Affected Modules
- `<path or module>`: why it was involved.

## Validation
- Tests run, tests added, commands, manual verification, and remaining gaps.

## Regression Guard
The test, rule, runbook, or checklist that should prevent recurrence.

## Rule Candidate
State whether this should become or update a Development Rule, and why.

## Unknowns
- Any residual uncertainty.
```

Bug fix records should be short enough to read during future debugging, but precise enough to prevent repeating the same investigation.

## Architecture Decision

Use for durable technical choices that future agents should not casually reverse.

Required sections:

```md
# <Decision Title>

## Background
Context and problem.

## Decision
The chosen direction.

## Alternatives Considered
- Alternative: benefit, cost, and why it was not chosen.

## Rationale
Tradeoffs, constraints, and evidence behind the decision.

## Impact
Affected modules, workflows, tests, and future maintenance.

## Follow-Up
Signals to watch, planned revisit points, or known limitations.
```

Architecture decisions should describe why, not just what.

## Development Rule

Use when a bug or review exposes a reusable engineering constraint.

Required sections:

```md
# <Rule Title>

## Rule
The constraint in one or two direct sentences.

## Applies When
The code areas, behaviors, or change types where this rule matters.

## Counterexample
What goes wrong when the rule is ignored.

## Recommended Practice
What to do instead.

## Source
Related bug records, decisions, incidents, or reviews.

## Verification
Tests, review checks, or manual validation that enforce the rule.
```

Promote a bug to a rule when it repeats, affects shared behavior, crosses platforms, or reveals a general UI/state/data-flow constraint.

## Short Examples

Development Spec style:

```md
# Adjustable Sidebar Density

## Background
The current sidebar takes too much space on small screens. Users need a compact mode without losing navigation access.

## Goals
- Persist a density setting.
- Apply compact spacing to sidebar navigation.

## Non-Goals
- Do not redesign the full navigation model.

## Affected Modules
- `settings`: store the new preference.
- `sidebar UI`: render compact spacing from the setting.

## Risks
- Reducing tap targets too far: keep a minimum hit area and verify keyboard navigation.

## Test Plan
- Settings default/load tests.
- UI tests for default and compact density.
```

Bug Fix Record style:

```md
# Search Field Does Not Focus

## Symptom
After opening search, the field appears but does not receive the cursor.

## Evidence
- The search field is conditionally rendered.
- Focus was attempted before the input was mounted.

## Root Cause
The code entered search mode and called focus synchronously, before the DOM node existed.

## Introduction History
Unconfirmed.

## Fix
Wait for the rendered input before focusing it, and allow the search trigger to toggle the mode off.

## Validation
- Focus path covered by a targeted UI/source test.
- Manual check: open search, type text, close search.

## Regression Guard
Conditional UI that needs focus must wait for the render cycle before calling DOM focus.
```

## Output Quality Checklist

Before finalizing, confirm:

- The chosen document type is clear.
- Evidence and conclusions are separated.
- Scope and affected modules are explicit.
- Risks map to validation.
- Unknowns are named instead of hidden.
- The document says whether knowledge should evolve.
