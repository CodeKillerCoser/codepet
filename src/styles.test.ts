import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

const styles = readFileSync(resolve(__dirname, "styles.css"), "utf8");

function blockFor(selector: string) {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`${escapedSelector}\\s*\\{([^}]+)\\}`));
  return match?.[1] ?? "";
}

describe("main app scrolling layout", () => {
  test("keeps app shell fixed while sidebar and content scroll independently", () => {
    expect(blockFor("body")).toContain("overflow: hidden");
    expect(blockFor(".app-shell")).toContain("height: 100vh");
    expect(blockFor(".app-shell")).toContain("overflow: hidden");
    expect(blockFor(".sidebar")).toContain("overflow-y: auto");
    expect(blockFor(".content")).toContain("overflow-y: auto");
  });
});
