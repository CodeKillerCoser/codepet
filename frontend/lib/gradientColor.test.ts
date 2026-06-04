import { describe, expect, it } from "vitest";
import {
  gradientCss,
  gradientSegmentCss,
  gradientEditorFromCss,
  nextGradientStopColor,
  runningBubbleStyle,
} from "./gradientColor";

describe("gradientColor", () => {
  it("builds a linear gradient css value from editor fields", () => {
    expect(gradientCss({ angle: 135, colors: ["#111111", "#777777", "#eeeeee"] })).toBe(
      "linear-gradient(135deg, #111111 0%, #777777 50%, #eeeeee 100%)",
    );
  });

  it("keeps one-stop color values as gradients so angle edits stay visible", () => {
    const css = gradientCss({ angle: 45, colors: ["#e8f2ff"] });

    expect(css).toBe("linear-gradient(45deg, #e8f2ff 0%, #e8f2ff 100%)");
    expect(gradientEditorFromCss(css, "#000000")).toEqual({
      angle: 45,
      colors: ["#e8f2ff"],
    });
  });

  it("builds a hard segmented preview band", () => {
    expect(gradientSegmentCss(["#00ffff", "#0088ff", "#ffd400", "#ff48aa"])).toBe(
      "linear-gradient(90deg, #00ffff 0%, #00ffff 25%, #0088ff 25%, #0088ff 50%, #ffd400 50%, #ffd400 75%, #ff48aa 75%, #ff48aa 100%)",
    );
  });

  it("keeps all selected color stops without an artificial limit", () => {
    const colors = [
      "#111111",
      "#222222",
      "#333333",
      "#444444",
      "#555555",
      "#666666",
      "#777777",
      "#888888",
      "#999999",
      "#aaaaaa",
    ];

    const css = gradientCss({ angle: 90, colors });
    const parsed = gradientEditorFromCss(css, "#000000");

    expect(parsed.colors).toEqual(colors);
  });

  it("generates a visibly different color for a newly added stop", () => {
    expect(nextGradientStopColor(["#ee2200"])).toBe("#0066ee");
    expect(nextGradientStopColor(["#336699", "#0066ee"])).toBe("#eecc00");
  });

  it("parses the supported linear gradient format for editing", () => {
    expect(gradientEditorFromCss("linear-gradient(45deg, #123456 0%, #abcdef 50%, #fedcba 100%)", "#000000")).toEqual({
      angle: 45,
      colors: ["#123456", "#abcdef", "#fedcba"],
    });
  });

  it("keeps solid colors editable as one-stop values", () => {
    expect(gradientEditorFromCss("#3d73d8", "#000000")).toEqual({
      angle: 90,
      colors: ["#3d73d8"],
    });
  });

  it("derives safe running bubble css variables from gradient colors", () => {
    const style = runningBubbleStyle({
      backgroundBreathing: true,
      borderMarquee: true,
      backgroundColor: "linear-gradient(120deg, #101828 0%, #7c3aed 100%)",
      borderColor: "linear-gradient(90deg, #22d3ee 0%, #f97316 100%)",
      borderWidth: 4,
      animationMs: 1600,
    });

    expect(style).toContain("--pet-running-bubble-bg: linear-gradient(120deg, #101828 0%, #7c3aed 100%)");
    expect(style).toContain("--pet-running-bubble-bg-dim: linear-gradient(120deg, color-mix(in srgb, #101828 88%, #22d3ee) 0%, color-mix(in srgb, #7c3aed 88%, #22d3ee) 100%)");
    expect(style).toContain("--pet-running-bubble-border: #22d3ee");
    expect(style).toContain("--pet-running-bubble-border-cool: #f97316");
    expect(style).toContain("--pet-running-bubble-border-width: 4px");
    expect(style).not.toContain("color-mix(in srgb, linear-gradient");
  });

  it("clamps running bubble marquee border width", () => {
    const narrow = runningBubbleStyle({
      backgroundBreathing: true,
      borderMarquee: true,
      backgroundColor: "#e8f2ff",
      borderColor: "#3d73d8",
      borderWidth: 0,
      animationMs: 1800,
    });
    const wide = runningBubbleStyle({
      backgroundBreathing: true,
      borderMarquee: true,
      backgroundColor: "#e8f2ff",
      borderColor: "#3d73d8",
      borderWidth: 20,
      animationMs: 1800,
    });

    expect(narrow).toContain("--pet-running-bubble-border-width: 1px");
    expect(wide).toContain("--pet-running-bubble-border-width: 8px");
  });
});
