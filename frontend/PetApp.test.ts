import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("Pet window behavior", () => {
  it("auto-hides transient pet notices after showing them", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("function showNotice");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("clearNoticeTimer");
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
