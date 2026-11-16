

export type ActivityGroupId = "attention" | "active" | "completed";

export interface ActivityGroup<T> {
  id: ActivityGroupId;
  label: string;
  activities: T[];
}

export function groupActivities<T extends { finished: boolean; actions: readonly { kind: string; enabled: boolean }[] }>(activities: T[]): ActivityGroup<T>[] {
  const groups: ActivityGroup<T>[] = [{ id: "attention", label: "等待交互", activities: [] },{ id: "active", label: "进行中", activities: [] },{ id: "completed", label: "已结束", activities: [] }];
  for (const activity of activities) { const group = activity.finished ? groups[2] : activity.actions.some(action => action.enabled && ['reply','permission'].includes(action.kind)) ? groups[0] : groups[1]; group.activities.push(activity); }
  return groups.filter(group => group.activities.length);
}
