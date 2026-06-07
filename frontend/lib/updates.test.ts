import { describe, expect, it } from "vitest";
import { ignoredUpdateSettings, shouldPromptForUpdate } from "./updates";
import type { AppUpdate } from "./types";

const update: AppUpdate = {
  version: "0.2.0",
  currentVersion: "0.1.0",
};

describe("shouldPromptForUpdate", () => {
  it("prompts manual checks even when the version was ignored for automatic checks", () => {
    expect(
      shouldPromptForUpdate(update, "manual", {
        updates: { ignoredVersion: "0.2.0" },
      }),
    ).toBe(true);
  });

  it("suppresses automatic checks for the ignored version", () => {
    expect(
      shouldPromptForUpdate(update, "auto", {
        updates: { ignoredVersion: "0.2.0" },
      }),
    ).toBe(false);
  });

  it("prompts automatic checks when a different version appears", () => {
    expect(
      shouldPromptForUpdate(update, "auto", {
        updates: { ignoredVersion: "0.1.5" },
      }),
    ).toBe(true);
  });

  it("records only the ignored version in update settings", () => {
    expect(ignoredUpdateSettings("0.2.0")).toEqual({ ignoredVersion: "0.2.0" });
  });
});
