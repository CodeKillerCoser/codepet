import { describe, expect, it } from "vitest";
import { colorStopIndexFromBand, updateRunningBubbleColorSetting } from "./bubbleColorSettings";
import type { AppSettings } from "./types";

function settingsFixture(): AppSettings {
  return {
    appearance: {
      theme: "system",
      runningBubble: {
        backgroundBreathing: true,
        borderMarquee: true,
        backgroundColor: "linear-gradient(90deg, #111111 0%, #222222 100%)",
        borderColor: "#333333",
        borderWidth: 1,
        animationMs: 1800,
      },
    },
    pet: {
      selectedPetId: "default",
      kind: "palette",
      sprite: { body: "#ffffff", accent: "#111111", eyes: "#000000" },
      imagePath: null,
      scale: 4,
      imagePixelSize: 48,
      alwaysOnTop: true,
    },
    petLibrary: {
      selectedPetId: "default",
      pets: [],
    },
    notifications: {
      sound: "blip",
      customSoundPath: null,
      ringOnPermission: true,
      ringOnFailure: true,
      ringOnDone: true,
      repeatSeconds: 0,
      quietHoursEnabled: false,
      quietHoursStart: "22:00",
      quietHoursEnd: "08:00",
    },
    activityFilters: {
      titleKeywords: [],
      messageKeywords: [],
    },
  };
}

describe("bubbleColorSettings", () => {
  it("returns new settings references so color editor previews update immediately", () => {
    const settings = settingsFixture();
    const next = updateRunningBubbleColorSetting(settings, "backgroundColor", "#e8f2ff", {
      colors: ["#111111", "#222222", "#00d084"],
    });

    expect(next).not.toBe(settings);
    expect(next.appearance).not.toBe(settings.appearance);
    expect(next.appearance.runningBubble).not.toBe(settings.appearance.runningBubble);
    expect(settings.appearance.runningBubble.backgroundColor).toBe("linear-gradient(90deg, #111111 0%, #222222 100%)");
    expect(next.appearance.runningBubble.backgroundColor).toBe(
      "linear-gradient(90deg, #111111 0%, #222222 50%, #00d084 100%)",
    );
  });

  it("keeps the active angle in the returned settings while the slider moves", () => {
    const next = updateRunningBubbleColorSetting(settingsFixture(), "backgroundColor", "#e8f2ff", {
      angle: 135,
    });

    expect(next.appearance.runningBubble.backgroundColor).toBe("linear-gradient(135deg, #111111 0%, #222222 100%)");
  });

  it("keeps angle changes visible even when the editor has one color stop", () => {
    const settings = settingsFixture();
    settings.appearance.runningBubble.backgroundColor = "#e8f2ff";

    const next = updateRunningBubbleColorSetting(settings, "backgroundColor", "#e8f2ff", {
      angle: 45,
    });

    expect(next.appearance.runningBubble.backgroundColor).toBe("linear-gradient(45deg, #e8f2ff 0%, #e8f2ff 100%)");
  });

  it("maps preview band clicks to the same color stop count shown by the editor", () => {
    expect(colorStopIndexFromBand(4, 0, 400)).toBe(0);
    expect(colorStopIndexFromBand(4, 120, 400)).toBe(1);
    expect(colorStopIndexFromBand(4, 260, 400)).toBe(2);
    expect(colorStopIndexFromBand(4, 399, 400)).toBe(3);
  });
});
