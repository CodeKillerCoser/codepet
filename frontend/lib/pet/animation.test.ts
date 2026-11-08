import { describe, expect, it } from "vitest";
import { actionFor, animations, frameAt, petSize, taskStatusForPet } from "./animation";

describe("pet animation contract", () => {
  it("never plays the empty trailing cells, including after long background pauses", () => {
    for (const action of Object.keys(animations) as (keyof typeof animations)[]) {
      for (const elapsed of [0, 139, 280, 820, 1100, 50000, 7000001]) {
        const frame = frameAt(action, elapsed);
        expect(frame.column).toBeLessThan(animations[action].durations.length);
        expect(frame.column).toBeGreaterThanOrEqual(0);
        expect(frame.nextIn).toBeGreaterThan(0);
      }
    }
    expect(frameAt("idle", 279).column).toBe(0);
    expect(frameAt("idle", 280).column).toBe(1);
    expect(frameAt("idle", 1100).column).toBe(0);
  });

  it("prioritizes required attention while allowing explicit dragging and petting", () => {
    expect(taskStatusForPet([{ status: "running" }, { status: "waiting-input" }])).toBe("waiting-approval");
    expect(taskStatusForPet([{ status: "running" }, { status: "failed" }])).toBe("failed");
    expect(actionFor("waiting-approval", "pet")).toBe("waiting");
    expect(actionFor("failed", "hover")).toBe("failed");
    expect(actionFor("idle", "pet")).toBe("celebrate");
    expect(actionFor("running", "drag-left")).toBe("walkLeft");
    expect(actionFor("running", "none")).toBe("running");
    expect(actionFor("done", "none")).toBe("celebrate");
  });

  it("contains invalid settings and preserves the atlas aspect ratio", () => {
    expect(petSize(NaN)).toEqual(petSize(3));
    expect(petSize(100)).toEqual({ width: 160, height: 173 });
    expect(petSize(-3)).toEqual(petSize(1));
  });
});
