# Reply Editor

## Current Behavior

Reply mode is controlled by `replyingToId` and `replyText` in `frontend/PetApp.svelte`.

When opened, the textarea is focused, cursor is moved to the end, and the editor height is adjusted from `scrollHeight`. Maximum height is five rows, calculated from computed line height, padding, and border.

Reply mode can be cancelled with the cancel button, submit clears it on success, and it is cleared when the target activity disappears or loses reply capability.

## Capability Dependency

The reply button is shown only when `activityCapabilities(activity).canReply` is true and the card is not already in reply mode.

## Risks

- Conditional rendering can mount the textarea after the click handler runs. Focus must happen after DOM update.
- Reusing one `replyTextarea` binding across list items is safe only while one card can be in reply mode.
- Autosize must not force the whole task card layout into a fixed height.

## Validation

- Manually click reply and verify the caret appears in the editor.
- Type more than five lines and verify the editor scrolls internally.
- Cancel reply and verify the footer action returns.
