import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("Pet window behavior", () => {
  it("keeps the Pet Gateway source while grouping the original compact bubbles", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const data = readFileSync(new URL('./lib/pet/model.ts', import.meta.url), 'utf8');
    const desktop = readFileSync(new URL('./lib/pet/DesktopPet.svelte', import.meta.url), 'utf8');
    expect(data).toContain("petSnapshot");
    expect(desktop).toContain('bind:expanded={expandedGroups} dismiss={dismissActivity}');
    expect(source).toContain(".activity-group-toggle, .status-pill, .pet-action-button");
    expect(source).toContain('element.closest("[inert]")');
    expect(source).not.toContain("recentEvents");
  });

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
    const source = readFileSync(new URL("./lib/pet/DesktopPet.svelte", import.meta.url), "utf8");

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
    const source = readFileSync(new URL("./lib/pet/DesktopPet.svelte", import.meta.url), "utf8");
    const stageBlock = source.slice(source.indexOf('<section class="pet-stage"'), source.indexOf("{#key whipAnimationKey}"));

    expect(stageBlock).toContain('<div class="pet-action-rail"');
    expect(stageBlock).toContain('class="pet-action-button fold-button"');
    expect(stageBlock).toContain('class="pet-action-button main-window-button"');
    expect(stageBlock).toContain('class="pet-action-button whip-button"');
    expect(stageBlock.indexOf('class="pet-action-button fold-button"')).toBeLessThan(stageBlock.indexOf('class="pet-action-button main-window-button"'));
    expect(stageBlock.indexOf('class="pet-action-button main-window-button"')).toBeLessThan(stageBlock.indexOf('class="pet-action-button whip-button"'));
  });



  it("lets CSS size the activity stack naturally up to a max height", () => {
    const source = readFileSync(new URL("./lib/pet/DesktopPet.svelte", import.meta.url), "utf8");
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const stackRule = styles.slice(styles.indexOf(".activity-stack"), styles.indexOf(".activity-stack::-webkit-scrollbar"));

    expect(source).toContain('style={`--pet-activity-stack-max-height: ${activityStackHeightLimit}px`}');
    expect(source).not.toContain("activityStackHeightFor");
    expect(source).not.toContain("activityCardHeight");
    expect(stackRule).toContain("height: auto");
    expect(stackRule).toContain("max-height: var(--pet-activity-stack-max-height)");
  });

  it("passes cursor events through transparent pet-window regions", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("pet_surface_pointer");
    expect(source).toContain("setIgnoreCursorEvents");
    expect(source).toContain("collectPetHitRects");
    expect(source).toContain("isPointOnOpaquePetImage");
    expect(source).toContain("shouldIgnorePetWindowCursor");
    expect(source).toContain("clearCursorPassthroughTimer");
    expect(source).toContain("{dragWindow}");
    expect(source).not.toContain('<button class="drag-layer"');
  });

  it("applies a clamped overall pet window opacity from settings", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("$: petWindowOpacity = clampPetOpacity(settings?.pet.opacity)");
    expect(source).toContain("const minPetOpacity = 0.25");
    expect(source).toContain("Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue))");
    expect(source).toContain('--pet-window-opacity: ${petWindowOpacity};');
  });


});
