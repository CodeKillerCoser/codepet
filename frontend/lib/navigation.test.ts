import { get } from "svelte/store";
import { expect, test } from "vitest";
import { createNavigation } from "./navigation";

test("page history records settings and restores the previous page without adding entries", () => {
  const history = createNavigation("tasks");
  expect(get(history).canGoBack).toBe(false);
  history.navigate("settings");
  history.back();
  expect(get(history)).toMatchObject({ current: "tasks", index: 0, canGoForward: true, entries: ["tasks", "settings"] });
  history.forward();
  expect(get(history)).toMatchObject({ current: "settings", index: 1, canGoForward: false });
});
test("new navigation truncates forward history and duplicate pages are ignored", () => {
  const history = createNavigation();
  history.navigate("tasks"); history.navigate("settings"); history.back(); history.navigate("usage"); history.navigate("usage");
  expect(get(history)).toMatchObject({ entries: ["agents", "tasks", "usage"], canGoForward: false });
});
test("history is bounded and movement clamps at either end", () => {
  const history = createNavigation();
  history.back(); expect(get(history).index).toBe(0);
  for (let i = 0; i < 110; i++) history.navigate(i % 2 ? "tasks" : "settings");
  history.forward(); expect(get(history).entries).toHaveLength(100); expect(get(history).index).toBe(99);
});
