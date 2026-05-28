import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

const styles = readFileSync(resolve(__dirname, "styles.css"), "utf8");
const petAppSource = readFileSync(resolve(__dirname, "PetApp.svelte"), "utf8");

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

describe("pet message bubble activity", () => {
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
    expect(marquee).toContain("padding-box");
    expect(marquee).toContain("border-box");
    expect(marquee).toContain("conic-gradient");
    expect(marquee).toContain("0deg");
    expect(marquee).toContain("360deg");
    expect(styles).not.toContain("stroke-dasharray");
    expect(styles).not.toContain("stroke-dashoffset");
    expect(styles).not.toContain("mask-composite");
  });

  test("uses a multi-stop gradient palette for marquee borders", () => {
    const marquee = blockFor(".status-pill.active-marquee");

    expect(petAppSource).toContain("--pet-running-bubble-border-hot");
    expect(petAppSource).toContain("--pet-running-bubble-border-cool");
    expect(petAppSource).toContain("--pet-running-bubble-border-light");
    expect(marquee).toContain("--pet-running-bubble-border-hot");
    expect(marquee).toContain("--pet-running-bubble-border-cool");
    expect(marquee).toContain("--pet-running-bubble-border-light");
    expect(marquee).toContain("72deg");
    expect(marquee).toContain("144deg");
    expect(marquee).toContain("216deg");
    expect(marquee).toContain("288deg");
  });

  test("uses distinct dim and peak colors for background breathing", () => {
    const keyframes = keyframesFor("pet-pill-breathe");

    expect(petAppSource).toContain("--pet-running-bubble-bg-dim");
    expect(petAppSource).toContain("--pet-running-bubble-bg-peak");
    expect(petAppSource).toContain("color-mix(in srgb, ${runningBubble.backgroundColor} 88%, ${runningBubble.borderColor})");
    expect(petAppSource).toContain("color-mix(in srgb, ${runningBubble.backgroundColor} 76%, white)");
    expect(keyframes).toContain("--pet-running-bubble-bg-dim");
    expect(keyframes).toContain("--pet-running-bubble-bg-peak");
    expect(petAppSource).not.toContain("72%, ${runningBubble.borderColor}");
    expect(petAppSource).not.toContain("34%, white");
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
