import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("PetApp activity helpers", () => {
  it("imports every activity helper used by the activity card template", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");
    const activityImport = source.match(/import\s+\{([^}]+)\}\s+from\s+"\.\/lib\/activity";/);

    expect(activityImport?.[1].split(",").map((name) => name.trim()).sort()).toEqual(
      expect.arrayContaining(["cardMeta"]),
    );
  });

  it("auto-hides transient pet notices after showing them", () => {
    const source = readFileSync(new URL("./PetApp.svelte", import.meta.url), "utf8");

    expect(source).toContain("function showNotice");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("clearNoticeTimer");
  });
});
