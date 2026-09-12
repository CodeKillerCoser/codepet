import { describe, expect, it } from "vitest";
import { groupActivities } from "./activityGroups";
import { updateActivityList } from "./activity";
import type { PetEvent, TaskStatus } from "./types";

function event(id: string, status: TaskStatus): PetEvent {
  return { id, sessionId: id, provider: "codex", kind: "task-updated", status, title: id, message: "", shouldRing: false, createdAt: new Date().toISOString(), raw: null };
}

describe("activity groups", () => {
  it("prioritizes attention, preserves order within groups and omits idle activities", () => {
    const input = [event("done", "done"), event("run", "running"), event("fail", "failed"), event("wait", "waiting-approval"), event("think", "thinking"), event("idle", "idle")];
    const groups = groupActivities(input);
    expect(groups.map((group) => [group.id, group.activities.map((item) => item.id)])).toEqual([
      ["attention", ["fail", "wait"]], ["active", ["run", "think"]], ["completed", ["done"]],
    ]);
    expect(input.map((item) => item.id)).toEqual(["done", "run", "fail", "wait", "think", "idle"]);
  });

  it("moves the same session between groups without duplicates as its status changes", () => {
    let activities = updateActivityList([], [event("task", "running")]);
    for (const status of ["waiting-approval", "running", "done"] as const) {
      activities = updateActivityList(activities, [{ ...event("task", status), id: `update-${status}` }]);
      const groups = groupActivities(activities);
      expect(groups).toHaveLength(1);
      expect(groups[0].id).toBe(status === "done" ? "completed" : status === "running" ? "active" : "attention");
      expect(groups[0].activities).toHaveLength(1);
    }
  });

  it("groups Pet Gateway waiting-input, completed and interrupted statuses", () => {
    const tasks = [{ id: "input", status: "waiting-input" }, { id: "done", status: "completed" }, { id: "cancel", status: "interrupted" }, { id: "unknown", status: "unknown" }];
    expect(groupActivities(tasks).map(group => [group.id, group.activities.map(task => task.id)])).toEqual([
      ["attention", ["input", "unknown"]], ["completed", ["done", "cancel"]],
    ]);
  });

  it("omits empty groups", () => {
    expect(groupActivities([])).toEqual([]);
    expect(groupActivities([event("done", "done")]).map((group) => group.id)).toEqual(["completed"]);
  });
});
