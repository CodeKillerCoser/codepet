import { describe, expect, test } from "vitest";
import { reachable, rootThread, taskStatus, type LineageTask, type LineageThread } from "./taskLineage";
const task = { id: "t", manualCompletion: false, episodes: [{ id: "a", threadId: "one" }, { id: "b", threadId: "two" }, { id: "c", threadId: "one" }], edges: [{ from: "a", to: "b" }, { from: "b", to: "c" }, { from: "a", to: "sibling" }] } as LineageTask;
describe("task lineage evidence views", () => {
  test("hover includes ancestors and descendants without unrelated siblings", () => { expect([...reachable(task, "b")].sort()).toEqual(["a", "b", "c"]); });
  test("unknown state cannot masquerade as ready for acceptance", () => {
    expect(taskStatus(task, { one: "idle" })).toBe("状态待确认");
    expect(taskStatus(task, { one: "idle", two: "running" })).toBe("进行中");
    expect(taskStatus(task, { one: "idle", two: "idle" })).toBe("待验收");
  });
  test("missing parent and cycles do not hang navigation", () => {
    expect(rootThread("a", [{ id: "a", parentId: "missing" }] as LineageThread[])).toBe("a");
    const threads = [{ id: "a", parentId: "b" }, { id: "b", parentId: "a" }] as LineageThread[];
    expect(rootThread("a", threads)).toBe(rootThread("b", threads));
  });
});
