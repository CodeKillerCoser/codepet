# Task Card Interaction

## Card Content

Task cards display a title, message, provider/source metadata, status, and terminal time when available. Display helpers live in `frontend/lib/activity.ts`.

## Activity Merge

Cards are grouped by provider plus session id, cwd, or a global fallback. Active task updates replace the prior card for the same activity key. Terminal events are retained only when they belong to an existing visible activity or can be matched by fallback.

## Actions

- Open: attempts to activate the source application or project path.
- Remove: removes any task card from the pet list.
- Remove completed: clears completed cards.
- Reply: only visible when provider capability says it is safe.
- Approval: visible only for waiting approval events.

## Reply Mode

Reply mode is local UI state in `frontend/PetApp.svelte`. It can be toggled open and closed, focuses the textarea, and resizes the editor up to five rows.

## Risks

- Adding footer actions can collapse spacing or remove the bottom card gap.
- Fixed card heights are fragile because reply editor, footer, and message content vary.
- Showing reply during running states is wrong because the backend cannot reliably inject into active provider sessions except approved control paths.

## Validation

- Run `npx vitest run frontend/lib/activity.test.ts`.
- Manually inspect one-card, multi-card, reply-mode, and approval-mode layouts when editing `frontend/PetApp.svelte`.
