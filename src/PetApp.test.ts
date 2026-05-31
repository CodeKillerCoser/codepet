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

  it("auto-expands the task list while live activities are present", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("hasLiveActivities");
    expect(source).toContain('activity.status === "running"');
    expect(source).toContain("hasLiveActivities && tasksCollapsed");
    expect(source).toContain("tasksCollapsed = false");
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
    expect(source).toContain('aria-label="抽鞭子"');
    expect(source).toContain('class="whip-button"');
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

  it("serializes pet window resize requests so stale small frames cannot win", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const syncWindowFrame = source.slice(source.indexOf("async function syncWindowFrame"), source.indexOf("function petWindowHeight"));

    expect(source).toContain("let requestedWindowHeight = 0");
    expect(source).toContain("let syncingWindowFrame = false");
    expect(syncWindowFrame).toContain("requestedWindowHeight = Math.round(height)");
    expect(syncWindowFrame).toContain("if (syncingWindowFrame) {");
    expect(syncWindowFrame).toContain("while (true)");
    expect(syncWindowFrame).toContain("const targetHeight = requestedWindowHeight");
    expect(syncWindowFrame).toContain("if (!applied || requestedWindowHeight === targetHeight)");
  });

  it("renders only the visible activity slice used for pet window sizing", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: visibleActivities = showActivities ? activities.slice(0, maxVisibleActivities) : []");
    expect(source).toContain("{#each visibleActivities as activity (activity.id)}");
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

    expect(source).toContain("$: activityStackHeight = activityStackHeightFor(visibleActivities, replyingToId)");
    expect(source).toContain("$: desiredWindowHeight = petWindowHeight(activityStackHeight, petStageHeight)");
    expect(source).toContain("function activityStackHeightFor");
    expect(source).toContain("activity.id === activeReplyingToId");
    expect(source).toContain('activity.status === "waiting-approval" ? 86 : activityCardHeight');
    expect(source).toContain('style={`--pet-activity-stack-height: ${activityStackHeight}px`}');
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
});
