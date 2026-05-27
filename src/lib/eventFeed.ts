import type { PetEvent } from "./types";

export function mergeEventFeed(current: PetEvent[], incoming: PetEvent[], maxItems = 200): PetEvent[] {
  if (incoming.length === 0) {
    return current;
  }

  const byId = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) {
    byId.set(event.id, event);
  }

  return Array.from(byId.values())
    .sort((first, second) => new Date(first.createdAt).getTime() - new Date(second.createdAt).getTime())
    .slice(-maxItems);
}
