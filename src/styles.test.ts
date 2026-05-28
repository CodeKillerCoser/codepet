import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

const styles = readFileSync(resolve(__dirname, "styles.css"), "utf8");

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
  test("uses stronger active color variables for the breathing state", () => {
    expect(blockFor(".status-pill.active-status")).toContain("background: var(--pet-active-bg)");
    expect(blockFor(".status-pill.active-status")).toContain("box-shadow");
    expect(blockFor(".pet-window.theme-light")).toContain("--pet-active-ring");
    expect(blockFor(".pet-window.theme-light")).toContain("--pet-active-glow");
    expect(blockFor(".pet-window.theme-dark")).toContain("--pet-active-ring");
    expect(blockFor(".pet-window.theme-dark")).toContain("--pet-active-glow");
  });

  test("does not resize the bubble during breathing", () => {
    expect(keyframesFor("pet-pill-breathe")).not.toContain("transform");
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
