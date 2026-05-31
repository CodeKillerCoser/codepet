import { describe, expect, it } from "vitest";
import { whipCrackAnimation } from "./whipLottie";

describe("whipCrackAnimation", () => {
  it("defines a short transparent lottie animation with curved whip paths", () => {
    expect(whipCrackAnimation.fr).toBeGreaterThanOrEqual(30);
    expect(whipCrackAnimation.op).toBeLessThanOrEqual(32);
    expect(whipCrackAnimation.w).toBeGreaterThan(whipCrackAnimation.h);
    expect(whipCrackAnimation.layers.length).toBeGreaterThanOrEqual(3);
    expect(JSON.stringify(whipCrackAnimation)).toContain("Whip Cord");
    expect(JSON.stringify(whipCrackAnimation)).toContain("Crack Flash");
  });
});
