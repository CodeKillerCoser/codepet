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
});
