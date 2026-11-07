import { describe, expect, it } from "vitest";
import { imagePixelAt } from "./hitTest";

describe("pet alpha coordinates", () => {
  const bounds = { left: 10, top: 20, right: 106, bottom: 124 };
  it("samples the displayed atlas cell, not the first frame or a neighboring row", () => {
    expect(imagePixelAt({ x: 58, y: 72 }, { bounds, width: 1536, height: 1872, columns: 8, rows: 9, column: 4, row: 6 })).toEqual({ x: 864, y: 1352 });
    expect(imagePixelAt({ x: 107, y: 72 }, { bounds, width: 1536, height: 1872, columns: 8, rows: 9 })).toBeNull();
  });
  it("passes clicks through the letterbox around a non-square imported photo", () => {
    const photo = { bounds: { left: 0, top: 0, right: 100, bottom: 100 }, width: 200, height: 100 };
    expect(imagePixelAt({ x: 50, y: 10 }, photo)).toBeNull();
    expect(imagePixelAt({ x: 50, y: 50 }, photo)).toEqual({ x: 100, y: 50 });
    expect(imagePixelAt({ x: 50, y: 75 }, photo)).toBeNull();
  });
});
