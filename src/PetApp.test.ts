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

    expect(source).toContain("const maxActivityStackHeight");
    expect(source).toContain("const maxPetStageHeight");
    expect(source).toContain("const petWindowPresetHeight");
    expect(source).toContain("const targetHeight = petWindowPresetHeight");
    expect(source).toContain("new LogicalSize(petWindowWidth, targetHeight)");
    expect(source).not.toContain("async function syncWindowFrame");
    expect(source).not.toContain("async function applyWindowFrame");
  });

  it("renders every activity while sizing the pet window from the first four cards", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: renderedActivities = showActivities ? activities : []");
    expect(source).toContain("$: stackSizedActivities = showActivities ? activities.slice(0, maxVisibleActivities) : []");
    expect(source).toContain("{#each renderedActivities as activity (activity.id)}");
    expect(source).not.toContain("{#each activities as activity (activity.id)}");
  });

  it("shows terminal activity times in the card footer row", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("cardEndTime");
    expect(source).toContain("{@const endedAt = cardEndTime(activity)}");
    expect(source).toContain('{#if endedAt}');
    expect(source).toContain('<span class="status-ended-at">{endedAt}</span>');
  });

  it("sizes the activity stack from visible card heights", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: activityStackHeight = activityStackHeightFor(stackSizedActivities, replyingToId)");
    expect(source).toContain("const petWindowPresetHeight");
    expect(source).toContain("function activityStackHeightFor");
    expect(source).toContain("activity.id === activeReplyingToId");
    expect(source).toContain('activity.status === "waiting-approval" ? 86 : activityCardHeight');
    expect(source).toContain('style={`--pet-activity-stack-height: ${activityStackHeight}px`}');
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

  it("keeps bottom spacing on activity cards with footer actions", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const actionCardRule = styles.slice(styles.indexOf(".status-pill:has(.reply-button)"), styles.indexOf(".status-pill:hover"));

    expect(actionCardRule).toContain("min-height: 86px");
    expect(actionCardRule).toContain("padding-bottom: 10px");
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

  it("does not toggle cursor passthrough for transparent pet-window regions", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).not.toContain("cursorPosition");
    expect(source).not.toContain("setIgnoreCursorEvents");
    expect(source).not.toContain("collectPetHitRects");
    expect(source).not.toContain("isPointOnOpaquePetImage");
    expect(source).not.toContain("shouldIgnorePetWindowCursor");
    expect(source).not.toContain("clearCursorPassthroughTimer");
    expect(source).toContain('data-pet-hit-target="stage"');
    expect(source).not.toContain('<button class="drag-layer"');
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
