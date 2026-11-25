import { describe, expect, it } from "vitest";
import { normalizeActivityFilters, normalizeRobotNotificationSettings, normalizeSettings } from "./model";
import { defaultRunningBubbleSettings } from "../theme";
import type { AppSettings } from "../types";

describe("settings compatibility after page extraction", () => {
  it("keeps notification defaults independent between old settings loads", () => {
    const first = normalizeRobotNotificationSettings(undefined);
    first.triggers.taskDone = false;
    first.channels.push({ provider: "feishu", id: "fixture", name: "渠道", enabled: true, webhookUrl: "", webhookSecret: "" });
    const second = normalizeRobotNotificationSettings(undefined);
    expect(second.enabled).toBe(false);
    expect(second.triggers.taskDone).toBe(true);
    expect(second.channels).toEqual([]);
  });

  it("preserves configured notification channels and custom message templates", () => {
    const result = normalizeRobotNotificationSettings({ enabled: true,
      template: { title: "自定义 {{status}}", header: "摘要", primary: "正文", secondary: "上下文", footer: "时间" },
      channels: [{ provider: "feishu", id: "fixture", name: "自定义渠道", enabled: false, webhookUrl: "https://example.invalid/hook", webhookSecret: "fixture" }],
    });
    expect(result.template.title).toBe("自定义 {{status}}");
    expect(result.channels[0]).toMatchObject({ id: "fixture", name: "自定义渠道", enabled: false, webhookUrl: "https://example.invalid/hook" });
  });

  it("does not spread legacy global filters over explicitly configured agent filters", () => {
    const result = normalizeActivityFilters({ titleKeywords: ["legacy"], byAgent: {
      codex: { titleKeywords: [" Task ", "task", ""], messageKeywords: ["保留"] },
    } });
    expect(result.byAgent?.codex).toEqual({ titleKeywords: ["Task"], messageKeywords: ["保留"] });
    expect(result.byAgent?.claude).toEqual({ titleKeywords: [], messageKeywords: [] });
  });

  it("bounds appearance values while retaining theme, imported pets and plugin directories", () => {
    // A serialized older installation may omit newer optional settings groups.
    const old = { appearance: { theme: "dark", runningBubble: { ...defaultRunningBubbleSettings, animationMs: 10, borderWidth: 50 } },
      pet: { selectedPetId: "custom", imagePath: "/fixture/pet.webp", opacity: 0.1 },
      petLibrary: { selectedPetId: "custom", pets: [] }, notifications: {},
      providerPlugins: { directories: ["/fixture/provider"] },
    } as unknown as AppSettings;
    const result = normalizeSettings(old);
    expect(result.appearance).toMatchObject({ theme: "dark", preset: "warm-stone", runningBubble: { animationMs: 600, borderWidth: 8 } });
    expect(result.pet).toMatchObject({ selectedPetId: "custom", imagePath: "/fixture/pet.webp", opacity: 0.25 });
    expect(result.providerPlugins.directories).toEqual(["/fixture/provider"]);
  });

  it("preserves valid theme presets and falls back to warm-stone for unknown values", () => {
    const base = {
      appearance: { theme: "light", preset: "forest-sage", runningBubble: { ...defaultRunningBubbleSettings } },
      pet: { selectedPetId: "default", opacity: 1 },
      petLibrary: { selectedPetId: "default", pets: [] },
      notifications: {},
      providerPlugins: { directories: [] },
    } as unknown as AppSettings;
    expect(normalizeSettings(structuredClone(base)).appearance.preset).toBe("forest-sage");
    const invalid = structuredClone(base);
    (invalid.appearance as { preset?: string }).preset = "unknown-theme";
    expect(normalizeSettings(invalid).appearance.preset).toBe("warm-stone");
  });
});
