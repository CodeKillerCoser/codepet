import { describe, expect, it } from "vitest";
import { wrappedDialogFocusIndex } from "./dialogFocus";

describe("dialog focus wrapping", () => {
  it("wraps forward Tab from the final control to the first control", () => {
    expect(wrappedDialogFocusIndex(2, 3, false)).toBe(0);
  });

  it("wraps reverse Tab from the first control to the final control", () => {
    expect(wrappedDialogFocusIndex(0, 3, true)).toBe(2);
  });

  it("moves focus back inside when no dialog control is active", () => {
    expect(wrappedDialogFocusIndex(-1, 3, false)).toBe(0);
    expect(wrappedDialogFocusIndex(-1, 3, true)).toBe(2);
  });

  it("leaves ordinary movement inside the dialog to the browser", () => {
    expect(wrappedDialogFocusIndex(1, 3, false)).toBeNull();
    expect(wrappedDialogFocusIndex(1, 3, true)).toBeNull();
  });
});
