// @vitest-environment jsdom
import { expect, test } from "vitest";
import { frontendBuild, needsPageReload, prepareReload, consumeReloadNotice } from "./webcontent";

test("same release with a new build reloads, identical build does not", () => {
  expect(needsPageReload(frontendBuild, { ...frontendBuild, canReload: true })).toBe(false);
  expect(needsPageReload(frontendBuild, { ...frontendBuild, builtAt: frontendBuild.builtAt + 1, canReload: true })).toBe(true);
  expect(needsPageReload(frontendBuild, { ...frontendBuild, version: "99.0.0", canReload: true })).toBe(true);
});
test("successful reload returns to About once without rewriting normal navigation", () => {
  expect(consumeReloadNotice()).toBe(false);
  prepareReload();
  expect(consumeReloadNotice()).toBe(true);
  expect(consumeReloadNotice()).toBe(false);
});
