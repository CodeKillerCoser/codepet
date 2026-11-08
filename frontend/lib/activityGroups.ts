

export type ActivityGroupId = "attention" | "active" | "completed";

export interface ActivityGroup<T> {
  id: ActivityGroupId;
  label: string;
  activities: T[];
}

export function groupActivities<T extends { status: string }>(activities: T[]): ActivityGroup<T>[] {
  const groups: ActivityGroup<T>[] = [
    { id: "attention", label: "等待交互", activities: [] },
    { id: "active", label: "进行中", activities: [] },
    { id: "completed", label: "已完成", activities: [] },
  ];
  for (const activity of activities) {
    const group = activity.status === "waiting-approval" || activity.status === "waiting-input" || activity.status === "unknown" || activity.status === "failed"
      ? groups[0]
      : activity.status === "running"
        ? groups[1]
        : (activity.status === "completed" || activity.status === "interrupted") ? groups[2] : null;
    group?.activities.push(activity);
  }
  return groups.filter((group) => group.activities.length > 0);
}
