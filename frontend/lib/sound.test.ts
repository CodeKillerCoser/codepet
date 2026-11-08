import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { playWhipSound, shouldPlayWhipReaction, whipReactionDelayMs } from "./sound";

describe("playWhipSound", () => {
  it("is available as a separate pet action sound", () => {
    expect(playWhipSound).toEqual(expect.any(Function));
  });

  it("bundles the three whip wav files through the frontend asset pipeline", () => {
    const root = resolve(import.meta.dirname, "../..");
    const source = readFileSync(resolve(root, "frontend/lib/sound.ts"), "utf8");
    const tauriConfig = readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8");
    const resources = [
      "resources/sounds/whip-crack.wav",
      "resources/sounds/whip-swing.wav",
      "resources/sounds/whip-heavy-crack.wav",
    ];

    for (const resource of resources) {
      expect(source).toContain(resource);
      expect(existsSync(resolve(root, "src-tauri", resource))).toBe(true);
    }
    expect(tauriConfig).toContain('"../webcontent/": "webcontent/"');
    expect(source).not.toContain("resolveResource");
  });
});

describe("whip reaction sound", () => {
  it("plays only configured pet reactions after the whip crack", () => {
    expect(shouldPlayWhipReaction("none")).toBe(false);
    expect(shouldPlayWhipReaction(null)).toBe(false);
    expect(shouldPlayWhipReaction("pa")).toBe(true);
    expect(shouldPlayWhipReaction("scream")).toBe(true);
    expect(shouldPlayWhipReaction("custom", null)).toBe(false);
    expect(shouldPlayWhipReaction("custom", "/tmp/ouch.wav")).toBe(true);
  });

  it("delays pet reaction audio until after the whip crack starts", () => {
    expect(whipReactionDelayMs("none")).toBe(0);
    expect(whipReactionDelayMs("pa")).toBeGreaterThan(100);
    expect(whipReactionDelayMs("scream")).toBeGreaterThan(100);
    expect(whipReactionDelayMs("custom", null)).toBe(0);
    expect(whipReactionDelayMs("custom", "/tmp/ouch.wav")).toBeGreaterThan(100);
  });
});
