import { describe, expect, it } from "vitest";
import { shouldIgnorePetWindowCursor, isOpaqueCssColor, rectFromElementBounds } from "./petHitTest";

describe("pet window hit testing", () => {
  it("ignores cursor events outside non-transparent hit rectangles", () => {
    const hitRects = [{ left: 176, top: 18, right: 280, bottom: 122 }];

    expect(shouldIgnorePetWindowCursor({ x: 90, y: 80 }, hitRects)).toBe(true);
    expect(shouldIgnorePetWindowCursor({ x: 220, y: 80 }, hitRects)).toBe(false);
  });

  it("treats transparent CSS colors as non-hit areas", () => {
    expect(isOpaqueCssColor("transparent")).toBe(false);
    expect(isOpaqueCssColor("rgba(0, 0, 0, 0)")).toBe(false);
    expect(isOpaqueCssColor("#f8fafc")).toBe(true);
  });

  it("converts element bounds into root-local hit rectangles with optional padding", () => {
    expect(rectFromElementBounds({ left: 20, top: 30, right: 70, bottom: 90 }, { left: 10, top: 20 }, 2)).toEqual({
      left: 8,
      top: 8,
      right: 62,
      bottom: 72,
    });
  });
});
