import { describe, expect, it } from "vitest";
import { atlasAnimationForStatus } from "./petAtlas";

describe("atlasAnimationForStatus", () => {
  it("maps task status to Codex pet atlas rows", () => {
    expect(atlasAnimationForStatus("idle").row).toBe(0);
    expect(atlasAnimationForStatus("thinking").row).toBe(8);
    expect(atlasAnimationForStatus("running").row).toBe(7);
    expect(atlasAnimationForStatus("waiting-approval").row).toBe(6);
    expect(atlasAnimationForStatus("failed").row).toBe(5);
    expect(atlasAnimationForStatus("done").row).toBe(0);
  });

  it("uses the Codex contract frame counts and durations", () => {
    expect(atlasAnimationForStatus("idle").frames).toEqual([
      { column: 0, durationMs: 280 },
      { column: 1, durationMs: 110 },
      { column: 2, durationMs: 110 },
      { column: 3, durationMs: 140 },
      { column: 4, durationMs: 140 },
      { column: 5, durationMs: 320 },
    ]);
    expect(atlasAnimationForStatus("running").frames).toHaveLength(6);
    expect(atlasAnimationForStatus("waiting-approval").frames.at(-1)).toEqual({ column: 5, durationMs: 260 });
  });
});
