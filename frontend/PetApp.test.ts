import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("PetApp activity helpers", () => {
  it("imports every activity helper used by the activity card template", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const activityImport = source.match(/import\s+\{([^}]+)\}\s+from\s+"\.\/lib\/activity";/);

    expect(activityImport?.[1].split(",").map((name) => name.trim()).sort()).toEqual(
      expect.arrayContaining(["cardMeta"]),
    );
  });

  it("auto-hides transient pet notices after showing them", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("function showNotice");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("clearNoticeTimer");
  });

  it("auto-expands the task list only when a new live activity appears", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const applyBlock = source.slice(source.indexOf("function applyIncomingEvents"), source.indexOf("function isActiveActivity"));

    expect(source).toContain("hasLiveActivities");
    expect(source).toContain('activity.status === "running"');
    expect(source).toContain("function isLiveActivity");
    expect(applyBlock).toContain("const previousLiveKeys = new Set(activities.filter(isLiveActivity).map(activityKey))");
    expect(applyBlock).toContain("const hasNewLiveActivity = nextActivities.some");
    expect(applyBlock).toContain("if (tasksCollapsed && hasNewLiveActivity)");
    expect(source).toContain("tasksCollapsed = false");
    expect(source).not.toContain("hasLiveActivities && tasksCollapsed");
  });

  it("prevents pet window double-click defaults", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("function preventPetWindowDoubleClick");
    expect(source).toContain("event.preventDefault()");
    expect(source).toContain("event.stopPropagation()");
    expect(source).toContain("on:dblclick={preventPetWindowDoubleClick}");
    expect(source).not.toContain("data-tauri-drag-region");
  });

  it("marks the pet window in dev mode for visual debugging", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("const devMode = import.meta.env.DEV");
    expect(source).toContain('devMode ? " dev-mode" : ""');
  });

  it("shows a whip action button that replays the braided demo whip and sound", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("playWhipSound");
    expect(source).toContain("playWhipSound(settings)");
    expect(source).toContain("function whipPet");
    expect(source).toContain("let whipAnimationKey = 0");
    expect(source).toContain("whipAnimationKey += 1");
    expect(source).toContain("{#key whipAnimationKey}");
    expect(source).toContain('aria-label="抽鞭子"');
    expect(source).toContain('class="pet-action-button whip-button"');
    expect(source).toContain('class="whip-animation whip-svg"');
    expect(source).toContain('class="whip-rig"');
    expect(source).toContain('class="handle-core"');
    expect(source).toContain('class="ferrule"');
    expect(source).toContain('class="join-knot"');
    expect(source).toContain('class="rope-core rope-thick"');
    expect(source).toContain('class="rope-strand light strand-mid"');
    expect(source).toContain('class="tail-line"');
    expect(source).not.toContain('class="whip-cord"');
    expect(source).not.toContain('d="M 148 24 C 114 26, 87 44, 66 72 S 33 120, 18 126"');
    expect(source).not.toContain("lottie.loadAnimation");
    expect(source).not.toContain("whipCrackAnimation");
  });

  it("shows a right-side action to clear completed activities", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const clearBlock = source.slice(source.indexOf("function clearCompletedActivities"), source.indexOf("async function activate"));

    expect(source).toContain("hasCompletedActivities");
    expect(source).toContain('aria-label="移除全部已完成任务"');
    expect(source).toContain('class="pet-action-button clear-completed-button"');
    expect(clearBlock).toContain('activity.status === "done"');
    expect(clearBlock).toContain("dismissedActivityKeys.add(key)");
    expect(clearBlock).toContain("replyingToId = null");
  });

  it("groups pet-side actions in one compact rail", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const stageBlock = source.slice(source.indexOf('<section class="pet-stage"'), source.indexOf("{#key whipAnimationKey}"));

    expect(stageBlock).toContain('<div class="pet-action-rail"');
    expect(stageBlock).toContain('class="pet-action-button fold-button"');
    expect(stageBlock).toContain('class="pet-action-button main-window-button"');
    expect(stageBlock).toContain('class="pet-action-button whip-button"');
    expect(stageBlock.indexOf('class="pet-action-button fold-button"')).toBeLessThan(stageBlock.indexOf('class="pet-action-button main-window-button"'));
    expect(stageBlock.indexOf('class="pet-action-button main-window-button"')).toBeLessThan(stageBlock.indexOf('class="pet-action-button whip-button"'));
  });

  it("uses a fixed preset maximum pet window size", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("const activityStackMaxHeight");
    expect(source).toContain("const maxPetStageHeight");
    expect(source).toContain("const petWindowPresetHeight");
    expect(source).toContain("const targetHeight = petWindowPresetHeight");
    expect(source).toContain("new LogicalSize(petWindowWidth, targetHeight)");
    expect(source).not.toContain("async function syncWindowFrame");
    expect(source).not.toContain("async function applyWindowFrame");
  });

  it("renders every activity and lets the stack viewport enforce overflow", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: renderedActivities = showActivities ? activities : []");
    expect(source).toContain("{#each renderedActivities as activity (activity.id)}");
    expect(source).not.toContain("{#each activities as activity (activity.id)}");
    expect(source).not.toContain("stackSizedActivities");
    expect(source).not.toContain("maxVisibleActivities");
  });

  it("shows terminal activity times in the card footer row", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("cardEndTime");
    expect(source).toContain("{@const endedAt = cardEndTime(activity)}");
    expect(source).toContain('{#if endedAt}');
    expect(source).toContain('<span class="status-ended-at">{endedAt}</span>');
  });

  it("uses the normalized source label in the footer agent text", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("cardAgentLabel");
    expect(source).toContain('<span class="status-agent">{cardAgentLabel(activity)}</span>');
    expect(source).not.toContain('<span class="status-agent">{activity.provider}</span>');
  });

  it("lets CSS size the activity stack naturally up to a max height", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const stackRule = styles.slice(styles.indexOf(".activity-stack"), styles.indexOf(".activity-stack::-webkit-scrollbar"));

    expect(source).toContain("const petWindowPresetHeight");
    expect(source).toContain('style={`--pet-activity-stack-max-height: ${activityStackMaxHeight}px`}');
    expect(source).not.toContain("activityStackHeightFor");
    expect(source).not.toContain("activityCardHeight");
    expect(stackRule).toContain("height: auto");
    expect(stackRule).toContain("max-height: var(--pet-activity-stack-max-height)");
  });

  it("clears reply mode when the source activity is no longer replyable", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const cleanupBlock = source.slice(source.indexOf("function clearReplyIfNoLongerAvailable"), source.indexOf("function applyIncomingEvents"));

    expect(source).toContain("$: clearReplyIfNoLongerAvailable(activities, replyingToId)");
    expect(cleanupBlock).toContain("activity.id === activeReplyingToId && activityCapabilities(activity).canReply");
    expect(cleanupBlock).toContain("replyingToId = null");
    expect(cleanupBlock).toContain('replyText = ""');
  });

  it("hides the footer reply button while the inline reply editor is open", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("{#if capabilities.canReply && replyingToId !== activity.id}");
  });

  it("focuses and auto-sizes the reply editor after entering reply mode", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const focusBlock = source.slice(source.indexOf("async function focusReplyEditor"), source.indexOf("function cancelReply"));
    const resizeBlock = source.slice(source.indexOf("function resizeReplyEditor"), source.indexOf("function replyEditorMaxHeight"));

    expect(source).toContain("let replyTextarea: HTMLTextAreaElement | null = null");
    expect(source).toContain("const replyEditorMaxRows = 5");
    expect(source).toContain("void focusReplyEditor(activity.id)");
    expect(focusBlock).toContain("await tick()");
    expect(focusBlock).toContain("await getCurrentWindow().setFocus()");
    expect(focusBlock).toContain("replyTextarea.focus({ preventScroll: true })");
    expect(resizeBlock).toContain("editor.style.height = \"auto\"");
    expect(resizeBlock).toContain("Math.min(editor.scrollHeight, maxHeight)");
  });

  it("uses a multiline reply editor with explicit cancel and keyboard exit", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const replyTemplate = source.slice(source.indexOf('<form class="reply-row"'), source.indexOf('<div class="status-footer"'));
    const keydownBlock = source.slice(source.indexOf("function handleReplyKeydown"), source.indexOf("function handleReplyInput"));

    expect(replyTemplate).toContain("<textarea");
    expect(replyTemplate).toContain("bind:this={replyTextarea}");
    expect(replyTemplate).toContain("on:mousedown={stopReplyEditorEvent}");
    expect(replyTemplate).toContain("on:input={handleReplyInput}");
    expect(replyTemplate).toContain("class=\"reply-cancel\"");
    expect(replyTemplate).toContain("on:click={cancelReply}");
    expect(keydownBlock).toContain('event.key === "Escape"');
    expect(keydownBlock).toContain("cancelReply(event)");
    expect(keydownBlock).toContain("event.ctrlKey || event.metaKey");
  });

  it("keeps reply mode pending until the backend confirms completion", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const sendBlock = source.slice(source.indexOf("async function sendReply"), source.indexOf("function handleReplyKeydown"));
    const replyTemplate = source.slice(source.indexOf('<form class="reply-row"'), source.indexOf('<div class="status-footer"'));

    expect(source).toContain("let replySubmitting = false");
    expect(sendBlock).toContain("replySubmitting = true");
    expect(sendBlock).toContain("await sendActivityReply(activity.id, message)");
    expect(sendBlock).toContain("replyingToId = null");
    expect(sendBlock).toContain("finally");
    expect(sendBlock).toContain("replySubmitting = false");
    expect(replyTemplate).toContain("disabled={replySubmitting || !replyText.trim()}");
    expect(replyTemplate).toContain('{replySubmitting ? "发送中" : "发送"}');
  });

  it("keeps reply editor pointer and focus events inside the textarea", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const replyTemplate = source.slice(source.indexOf('<form class="reply-row"'), source.indexOf('<div class="status-footer"'));
    const editorRule = styles.slice(styles.indexOf(".reply-row textarea"), styles.indexOf(".reply-row textarea::placeholder"));

    expect(source).toContain("function stopReplyEditorEvent");
    expect(replyTemplate).toContain("on:pointerdown={stopReplyEditorEvent}");
    expect(replyTemplate).toContain("on:mousedown={stopReplyEditorEvent}");
    expect(replyTemplate).toContain("on:click={stopReplyEditorEvent}");
    expect(replyTemplate).toContain("on:focus={stopReplyEditorEvent}");
    expect(editorRule).toContain("user-select: text");
  });

  it("keeps reply editor vertical padding balanced", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const replyRowRule = styles.slice(styles.indexOf(".reply-row"), styles.indexOf(".reply-row textarea"));

    expect(replyRowRule).toContain("padding: 3px 0");
    expect(replyRowRule).not.toContain("padding: 3px 0 2px");
  });

  it("places reply controls below the editor instead of squeezing the input row", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const replyRowRule = styles.slice(styles.indexOf(".reply-row"), styles.indexOf(".reply-row textarea"));
    const controlsRule = styles.slice(styles.indexOf(".reply-controls"), styles.indexOf(".reply-row button"));

    expect(replyRowRule).toContain("grid-template-columns: minmax(0, 1fr)");
    expect(controlsRule).toContain("justify-self: end");
  });

  it("keeps footer bottom spacing consistent between reply states", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const contentRule = styles.slice(styles.indexOf(".status-content"), styles.indexOf(".status-pill.replying .status-content"));
    const footerRule = styles.slice(styles.indexOf(".status-footer {"), styles.indexOf(".status-footer.with-actions"));
    const actionCardRule = styles.slice(styles.indexOf(".status-pill:has(.reply-button)"), styles.indexOf(".status-pill:hover"));

    expect(actionCardRule).toContain("min-height: 86px");
    expect(actionCardRule).not.toContain("padding-bottom");
    expect(contentRule).toContain("align-self: stretch");
    expect(footerRule).toContain("margin-top: auto");
  });

  it("keeps every activity bubble the same width while capping the reply editor to five rows", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const stackRule = styles.slice(styles.indexOf(".activity-stack"), styles.indexOf(".activity-stack::-webkit-scrollbar"));
    const pillRule = styles.slice(styles.indexOf(".status-pill"), styles.indexOf(".status-pill.urgent"));
    const replyingRule = styles.slice(styles.indexOf(".status-pill.replying"), styles.indexOf(".status-content"));
    const editorRule = styles.slice(styles.indexOf(".reply-row textarea"), styles.indexOf(".reply-row textarea::placeholder"));

    expect(stackRule).toContain("width: 326px");
    expect(pillRule).toContain("width: 316px");
    expect(replyingRule).toContain("z-index: 6");
    expect(replyingRule).toContain("min-height: 154px");
    expect(replyingRule).toContain("align-items: stretch");
    expect(replyingRule).not.toContain("width:");
    expect(editorRule).toContain("max-height: 92px");
    expect(editorRule).toContain("resize: none");
    expect(editorRule).toContain("overscroll-behavior: contain");
  });

  it("renders approval actions for waiting approval activities", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("{#if capabilities.canApprove}");
    expect(source).toContain('on:click={(event) => approve(event, activity, "allow")}');
    expect(source).toContain('on:click={(event) => approve(event, activity, "deny")}');
  });

  it("allows dismissing activities in any status", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const titleRowBlock = source.slice(source.indexOf('<div class="status-title-row">'), source.indexOf('<button class="status-open" type="button"'));

    expect(titleRowBlock).toContain('class="dismiss-button inline-dismiss"');
    expect(titleRowBlock).toContain("dismissActivity(event, activity)");
    expect(titleRowBlock).not.toContain('activity.status === "done" || activity.status === "failed"');
  });

  it("deduplicates pushed pet events before ringing", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const listenBlock = source.slice(source.indexOf('listen<PetEvent>("pet-event"'), source.indexOf('listen<AppSettings>("settings-updated"'));

    expect(listenBlock).toContain("const alreadySeen = seenEventIds.has(event.payload.id) || event.payload.id === lastEventId");
    expect(listenBlock).toContain("if (!alreadySeen) {");
    expect(listenBlock).toContain("await handleRing(event.payload)");
  });

  it("stops repeated permission rings when the source activity is dismissed or expires", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const handleRingBlock = source.slice(source.indexOf("async function handleRing"), source.indexOf("function clearRepeat"));
    const clearRepeatBlock = source.slice(source.indexOf("function clearRepeat"), source.indexOf("function clearPoll"));
    const dismissBlock = source.slice(source.indexOf("function dismissActivity"), source.indexOf("async function activate"));

    expect(source).toContain("const permissionRepeatMaxMs = 590_000");
    expect(source).toContain("let repeatEventId: string | null = null");
    expect(source).toContain("let repeatExpiresAt = 0");
    expect(handleRingBlock).toContain("repeatEventId = event.id");
    expect(handleRingBlock).toContain("repeatExpiresAt = Date.now() + permissionRepeatMaxMs");
    expect(handleRingBlock).toContain("shouldRepeatNotification(settings, repeatEvent, activities, Date.now(), repeatExpiresAt)");
    expect(dismissBlock).toContain("repeatEvent && activityKey(repeatEvent) === activityKey(activity)");
    expect(dismissBlock).toContain("clearRepeat()");
    expect(clearRepeatBlock).toContain("repeatEventId = null");
    expect(clearRepeatBlock).toContain("repeatExpiresAt = 0");
  });

  it("keeps repeat ringing tied to the current waiting-approval activity", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const applyBlock = source.slice(source.indexOf("function applyIncomingEvents"), source.indexOf("function isActiveActivity"));
    const handleRingBlock = source.slice(source.indexOf("async function handleRing"), source.indexOf("function clearRepeat"));
    const repeatGuardBlock = source.slice(source.indexOf("function stopRepeatIfNoLongerNeedsAttention"), source.indexOf("async function dockToLowerRight"));

    expect(source).toContain("shouldRepeatNotification");
    expect(source).toContain("let repeatEvent: PetEvent | null = null");
    expect(applyBlock).toContain("stopRepeatIfNoLongerNeedsAttention()");
    expect(handleRingBlock).toContain("repeatEvent = event");
    expect(handleRingBlock).toContain("shouldRepeatNotification(settings, repeatEvent, activities, Date.now(), repeatExpiresAt)");
    expect(repeatGuardBlock).toContain("shouldRepeatNotification(settings, repeatEvent, activities, Date.now(), repeatExpiresAt)");
    expect(repeatGuardBlock).toContain("clearRepeat()");
  });

  it("clears repeat ringing by activity key when the visible permission card is dismissed", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const dismissBlock = source.slice(source.indexOf("function dismissActivity"), source.indexOf("async function activate"));

    expect(dismissBlock).toContain("repeatEvent && activityKey(repeatEvent) === activityKey(activity)");
    expect(dismissBlock).not.toContain("repeatEventId === activity.id");
  });

  it("passes cursor events through transparent pet-window regions", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("cursorPosition");
    expect(source).toContain("setIgnoreCursorEvents");
    expect(source).toContain("collectPetHitRects");
    expect(source).toContain("isPointOnOpaquePetImage");
    expect(source).toContain("shouldIgnorePetWindowCursor");
    expect(source).toContain("clearCursorPassthroughTimer");
    expect(source).toContain('data-pet-hit-target="stage"');
    expect(source).not.toContain('<button class="drag-layer"');
  });

  it("applies a clamped overall pet window opacity from settings", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: petWindowOpacity = clampPetOpacity(settings?.pet.opacity)");
    expect(source).toContain("const minPetOpacity = 0.25");
    expect(source).toContain("Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue))");
    expect(source).toContain('style={`--pet-window-opacity: ${petWindowOpacity};`}');
  });

  it("keeps the pet window frame and bounds stable after monitor moves or resizes", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const ensureBlock = source.slice(source.indexOf("async function ensureWindowFrameAndBounds"), source.indexOf("function withTimeout"));

    expect(source).toContain("onMoved");
    expect(source).toContain("onResized");
    expect(source).toContain("setResizable(false)");
    expect(source).toContain("scheduleEnsureWindowFrameAndBounds");
    expect(source).toContain("clearEnsureWindowFrameTimer");
    expect(ensureBlock).toContain("ensureWindowSize");
    expect(ensureBlock).toContain("constrainWindowToScreen");
    expect(ensureBlock).toContain("outerPosition()");
    expect(ensureBlock).toContain("outerSize()");
    expect(ensureBlock).toContain("monitorForWindow");
    expect(ensureBlock).toContain("clampWindowPositionToMonitor");
    expect(ensureBlock).toContain("monitor.workArea");
    expect(ensureBlock).toContain("currentSize.toLogical(scaleFactor)");
    expect(source).not.toContain("workArea.position.y >= 0");
  });
});
