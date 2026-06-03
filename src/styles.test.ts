import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

const styles = readFileSync(resolve(__dirname, "styles.css"), "utf8");
const themeTokens = readFileSync(resolve(__dirname, "lib/theme/tokens.css"), "utf8");
const appSource = readFileSync(resolve(__dirname, "App.svelte"), "utf8");
const petAppSource = readFileSync(resolve(__dirname, "PetApp.svelte"), "utf8");
const petSpriteSource = readFileSync(resolve(__dirname, "lib/PetSprite.svelte"), "utf8");
const gradientColorSource = readFileSync(resolve(__dirname, "lib/gradientColor.ts"), "utf8");

function blockFor(selector: string) {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`${escapedSelector}\\s*\\{([^}]+)\\}`));
  return match?.[1] ?? "";
}

function keyframesFor(name: string) {
  const start = styles.indexOf(`@keyframes ${name}`);
  if (start === -1) return "";
  const nextKeyframes = styles.indexOf("@keyframes", start + 1);
  const nextMedia = styles.indexOf("@media", start + 1);
  const endCandidates = [nextKeyframes, nextMedia].filter((index) => index !== -1);
  const end = endCandidates.length ? Math.min(...endCandidates) : styles.length;
  return styles.slice(start, end);
}

describe("main app scrolling layout", () => {
  test("keeps app shell fixed while sidebar and content scroll independently", () => {
    expect(blockFor("body")).toContain("overflow: hidden");
    expect(blockFor(".app-shell")).toContain("height: 100vh");
    expect(blockFor(".app-shell")).toContain("overflow: hidden");
    expect(blockFor(".sidebar")).toContain("overflow-y: auto");
    expect(blockFor(".content")).toContain("overflow-y: auto");
  });
});

describe("theme tokens", () => {
  test("loads Radix primitive palettes and exposes project semantic tokens", () => {
    expect(themeTokens).toContain('@import "@radix-ui/colors/gray.css"');
    expect(themeTokens).toContain('@import "@radix-ui/colors/gray-dark.css"');
    expect(themeTokens).toContain("--color-text-primary");
    expect(themeTokens).toContain("--font-family-ui");
    expect(themeTokens).toContain("--pet-pill-bg");
  });

  test("routes app and pet theme classes through the theme library", () => {
    expect(appSource).toContain("themeClassNames(");
    expect(petAppSource).toContain("themeClassNames(");
    expect(appSource).not.toContain('? "theme-dark" : "theme-light"');
    expect(petAppSource).not.toContain('? "theme-dark" : "theme-light"');
  });

  test("keeps production css free of raw theme colors", () => {
    expect(styles).not.toMatch(/#[0-9a-fA-F]{3,8}|rgba?\(/);
  });

  test("keeps production frontend sources on theme defaults and css tokens", () => {
    const productionSources = [appSource, petAppSource, petSpriteSource, gradientColorSource];
    for (const source of productionSources) {
      expect(source).not.toMatch(/(^|[^{}])#[0-9a-fA-F]{3,8}|rgba?\(/m);
    }
    expect(petAppSource).not.toMatch(/stop-color="#/);
  });

  test("keeps production css typography on theme tokens", () => {
    expect(styles).not.toMatch(/font-size:\s*[0-9]/);
    expect(styles).not.toMatch(/font-weight:\s*[0-9]/);
    expect(styles).not.toMatch(/line-height:\s*[0-9]/);
    expect(styles).not.toMatch(/letter-spacing:\s*[0-9]/);
  });
});

describe("pet message bubble activity", () => {
  test("shows a dev-only background on the pet window", () => {
    const devWindow = blockFor(".pet-window.dev-mode");

    expect(devWindow).toContain("background:");
    expect(devWindow).toContain("border:");
  });

  test("lets the pet activity stack size naturally up to its tokenized max height", () => {
    const stack = blockFor(".activity-stack");

    expect(stack).toContain("flex: 0 0 auto");
    expect(stack).toContain("height: auto");
    expect(stack).toContain("max-height: var(--pet-activity-stack-max-height)");
    expect(stack).toContain("overflow-y: auto");
    expect(stack).not.toContain("overflow-y: hidden");
  });

  test("exposes a marquee border width control", () => {
    const bubbleEditor = appSource.slice(appSource.indexOf('<section class="bubble-editor'), appSource.indexOf('<section class="appearance-editor', appSource.indexOf('<section class="bubble-editor')));

    expect(bubbleEditor).toContain("边框宽度");
    expect(bubbleEditor).toContain("runningBubbleBorderWidthLabel");
    expect(bubbleEditor).toContain("bind:value={settings.appearance.runningBubble.borderWidth}");
  });

  test("uses background-only hover styling for pet task cards", () => {
    const hover = blockFor(".status-pill:hover");

    expect(hover).toContain("background:");
    expect(hover).not.toContain("transform");
  });

  test("uses native color inputs as color stops for task bubble color bands", () => {
    const bubbleEditor = appSource.slice(appSource.indexOf('<section class="bubble-editor'), appSource.indexOf('<section class="appearance-editor', appSource.indexOf('<section class="bubble-editor')));

    expect(bubbleEditor).toContain("color-band-preview");
    expect(bubbleEditor).toContain("selectBubbleColorStopFromBand");
    expect(bubbleEditor).toContain('type="color"');
    expect(bubbleEditor).not.toContain("ColorPalette");
  });

  test("edits only one color stop at a time under the preview band", () => {
    const bubbleEditor = appSource.slice(appSource.indexOf('<section class="bubble-editor'), appSource.indexOf('<section class="appearance-editor', appSource.indexOf('<section class="bubble-editor')));
    const stopEditor = blockFor(".color-stop-editor");
    const stop = blockFor(".color-stop");
    const add = blockFor(".add-color-stop");

    expect(bubbleEditor).toContain("gradientSegmentCss");
    expect(bubbleEditor).toContain("selectedColorIndex");
    expect(bubbleEditor).toContain("{#key `${colorConfig.key}-${selectedColorIndex}-${editor.colors[selectedColorIndex]}`}");
    expect(blockFor(".color-band-preview")).toContain("cursor: pointer");
    expect(stopEditor).toContain("align-items: center");
    expect(stopEditor).toContain("flex-wrap: nowrap");
    expect(stop).toContain("width: 34px");
    expect(stop).toContain("height: 34px");
    expect(add).toContain("flex: 0 0 auto");
  });

  test("updates the background angle while the slider is moving", () => {
    const bubbleEditor = appSource.slice(appSource.indexOf('<section class="bubble-editor'), appSource.indexOf('<section class="appearance-editor', appSource.indexOf('<section class="bubble-editor')));

    expect(bubbleEditor).toContain("on:input={(event) => updateBubbleColor");
    expect(bubbleEditor).not.toContain("on:change={(event) => updateBubbleColor");
  });

  test("reassigns settings when task bubble colors change", () => {
    const bubbleEditorScript = appSource.slice(appSource.indexOf("function updateBubbleColor("), appSource.indexOf("function updateBubbleColorStop("));

    expect(bubbleEditorScript).toContain("settings = updateRunningBubbleColorSetting");
    expect(bubbleEditorScript).not.toContain("settings.appearance.runningBubble[key] =");
  });

  test("keeps newer bubble edits from being overwritten by older saves", () => {
    const saveScript = appSource.slice(appSource.indexOf("async function saveRunningBubbleSettings()"), appSource.indexOf("async function savePetImagePixelSize()"));

    expect(saveScript).toContain("runningBubbleSaveToken");
    expect(saveScript).toContain("saveToken !== runningBubbleSaveToken");
    expect(saveScript).not.toContain("await saveSettings()");
  });

  test("debounces bubble color saves so angle drags do not emit stale settings", () => {
    const updateScript = appSource.slice(appSource.indexOf("function updateBubbleColor("), appSource.indexOf("function updateBubbleColorStop("));
    const scheduleScript = appSource.slice(appSource.indexOf("function scheduleRunningBubbleSettingsSave()"), appSource.indexOf("async function saveRunningBubbleSettings()"));

    expect(updateScript).toContain("scheduleRunningBubbleSettingsSave()");
    expect(updateScript).not.toContain("void saveRunningBubbleSettings()");
    expect(scheduleScript).toContain("window.clearTimeout");
    expect(scheduleScript).toContain("window.setTimeout");
  });

  test("exposes settings and selected color stop dependencies directly in the template", () => {
    const bubbleEditor = appSource.slice(appSource.indexOf('<section class="bubble-editor'), appSource.indexOf('<section class="appearance-editor', appSource.indexOf('<section class="bubble-editor')));

    expect(bubbleEditor).toContain("settings.appearance.runningBubble[colorConfig.key]");
    expect(bubbleEditor).toContain("selectedBubbleColorStop[colorConfig.key]");
  });

  test("separates active bubble breathing and marquee animations", () => {
    expect(blockFor(".status-pill.active-breath")).toContain("pet-pill-breathe");
    expect(blockFor(".status-pill.active-marquee")).toContain("pet-pill-border-marquee");
    expect(blockFor(".status-pill.active-status")).not.toContain("pet-pill-breathe");
  });

  test("runs breathing and marquee animations together when both are enabled", () => {
    const combined = blockFor(".status-pill.active-breath.active-marquee");

    expect(combined).toContain("pet-pill-breathe");
    expect(combined).toContain("pet-pill-border-marquee");
  });

  test("does not resize the bubble during breathing", () => {
    expect(keyframesFor("pet-pill-breathe")).not.toContain("transform");
  });

  test("does not add shadow or glow to background breathing", () => {
    expect(blockFor(".status-pill.active-status")).not.toContain("--pet-active-glow");
    expect(blockFor(".status-pill.active-status")).not.toContain("--pet-active-shadow");
    expect(keyframesFor("pet-pill-breathe")).not.toContain("box-shadow");
    expect(keyframesFor("pet-pill-breathe")).not.toContain("glow");
  });

  test("keeps marquee as the bubble border layer with a continuous closed gradient", () => {
    const marquee = blockFor(".status-pill.active-marquee");

    expect(petAppSource).not.toContain("status-marquee");
    expect(marquee).toContain("border-width: var(--pet-running-bubble-border-width");
    expect(marquee).toContain("padding-box");
    expect(marquee).toContain("border-box");
    expect(marquee).toContain("conic-gradient");
    expect(marquee).toContain("0deg");
    expect(marquee).toContain("360deg");
    expect(marquee).not.toContain("stroke-dasharray");
    expect(marquee).not.toContain("stroke-dashoffset");
    expect(styles).not.toContain("mask-composite");
  });

  test("uses a multi-stop gradient palette for marquee borders", () => {
    const marquee = blockFor(".status-pill.active-marquee");

    expect(gradientColorSource).toContain("--pet-running-bubble-border-hot");
    expect(gradientColorSource).toContain("--pet-running-bubble-border-cool");
    expect(gradientColorSource).toContain("--pet-running-bubble-border-light");
    expect(marquee).toContain("--pet-running-bubble-border-hot");
    expect(marquee).toContain("--pet-running-bubble-border-cool");
    expect(marquee).toContain("--pet-running-bubble-border-light");
    expect(marquee).toContain("72deg");
    expect(marquee).toContain("144deg");
    expect(marquee).toContain("216deg");
    expect(marquee).toContain("288deg");
  });

  test("exposes whip reaction sound choices in personalization", () => {
    expect(appSource).toContain("settings.pet.whipReactionSound");
    expect(appSource).toContain("settings.pet.customWhipReactionSoundPath");
    expect(appSource).toContain("playWhipReactionSound(settings.pet.whipReactionSound, settings.pet.customWhipReactionSoundPath)");
    expect(appSource).toContain("pickCustomWhipReactionSound");
    expect(appSource).toContain("啪");
    expect(appSource).toContain("啊啊啊");
    expect(appSource).toContain("自定义");
  });

  test("exposes pet window opacity as a personalization control", () => {
    const petWindow = blockFor(".pet-window");

    expect(appSource).toContain("settings.pet.opacity");
    expect(appSource).toContain("petOpacityLabel(settings.pet.opacity)");
    expect(appSource).toContain("savePetOpacity");
    expect(petAppSource).toContain("--pet-window-opacity");
    expect(petWindow).toContain("opacity: var(--pet-window-opacity, 1)");
  });

  test("uses distinct dim and peak colors for background breathing", () => {
    const keyframes = keyframesFor("pet-pill-breathe");

    expect(gradientColorSource).toContain("--pet-running-bubble-bg-dim");
    expect(gradientColorSource).toContain("--pet-running-bubble-bg-peak");
    expect(gradientColorSource).toContain("88%, ${backgroundBorderColor}");
    expect(gradientColorSource).toContain("76%, ${cssColorTokens.white}");
    expect(keyframes).toContain("--pet-running-bubble-bg-dim");
    expect(keyframes).toContain("--pet-running-bubble-bg-peak");
    expect(gradientColorSource).not.toContain("72%, ${runningBubble.borderColor}");
    expect(gradientColorSource).not.toContain("34%, white");
    expect(keyframes).not.toContain("78%, white");
  });

  test("applies background breathing through the bubble background only", () => {
    const breath = blockFor(".status-pill.active-breath");
    const keyframes = keyframesFor("pet-pill-breathe");

    expect(styles).toContain("@property --pet-running-bubble-surface");
    expect(styles).toContain('syntax: "<color>"');
    expect(breath).toContain("--pet-running-bubble-surface");
    expect(breath).toContain("background:");
    expect(keyframes).not.toContain("border-color");
  });
});

describe("usage chart tooltip", () => {
  test("keeps tooltip inside the chart viewport", () => {
    const tooltip = blockFor(".usage-tooltip");
    expect(tooltip).toContain("top: 10px");
    expect(tooltip).toContain("width: max-content");
    expect(tooltip).not.toContain("bottom: calc(100% + 10px)");
  });
});
