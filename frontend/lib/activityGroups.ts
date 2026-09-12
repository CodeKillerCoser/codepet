import type { PetEvent } from "./types";

export type ActivityGroupId = "attention" | "active" | "completed";

export interface ActivityGroup {
  id: ActivityGroupId;
  label: string;
  activities: PetEvent[];
}

export function groupActivities(activities: PetEvent[]): ActivityGroup[] {
  const groups: ActivityGroup[] = [
    { id: "attention", label: "等待交互", activities: [] },
    { id: "active", label: "进行中", activities: [] },
    { id: "completed", label: "已完成", activities: [] },
  ];
  for (const activity of activities) {
    const group = activity.status === "waiting-approval" || activity.status === "failed"
      ? groups[0]
      : activity.status === "thinking" || activity.status === "running"
        ? groups[1]
        : activity.status === "done" ? groups[2] : null;
    group?.activities.push(activity);
  }
  return groups.filter((group) => group.activities.length > 0);
}
