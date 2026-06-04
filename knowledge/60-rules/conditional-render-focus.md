# Conditional Render Focus

## Rule

When focusing an element that is conditionally rendered, wait until the DOM has mounted that element before calling focus or setting selection.

## Applies When

- Svelte `{#if}` blocks mount inputs, textareas, buttons, or dialogs.
- A click handler toggles state and immediately tries to focus a newly rendered element.
- Only one element binding is reused across a list.

## Counterexample

Clicking a reply button sets `replyingToId`, but focus runs before the textarea exists. The user sees reply mode but cannot type immediately.

## Recommended Practice

Use Svelte's DOM update boundary before focus. Keep a clear reset path when the target item disappears or loses capability.

## Source

Reply editor focus regression in the pet task card.

## Verification

Manual click-to-reply check plus `frontend/PetApp.svelte` review when changing conditional inputs.
