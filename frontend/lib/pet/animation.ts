import type { TaskStatus } from "../types";

// Codex-compatible 8 × 9 atlas contract. Empty trailing cells are never played.
export const atlas = { columns: 8, rows: 9, frameWidth: 192, frameHeight: 208 } as const;
export const animations = {
  idle: { row: 0, durations: [280, 110, 110, 140, 140, 320] },
  walkRight: { row: 1, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  walkLeft: { row: 2, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  wave: { row: 3, durations: [180, 140, 140, 240] },
  celebrate: { row: 4, durations: [180, 140, 140, 140, 240] },
  failed: { row: 5, durations: [140, 140, 140, 140, 140, 140, 140, 240] },
  waiting: { row: 6, durations: [150, 150, 150, 150, 150, 260] },
  running: { row: 7, durations: [120, 120, 120, 120, 120, 220] },
  thinking: { row: 8, durations: [150, 150, 150, 150, 150, 280] },
} as const;
export type PetAction = keyof typeof animations;
export type PetInteraction = "none" | "hover" | "pet" | "drag-left" | "drag-right";

export function actionFor(status: TaskStatus, interaction: PetInteraction, startled = false): PetAction {
  if (startled) return "failed";
  if (interaction === "drag-left") return "walkLeft";
  if (interaction === "drag-right") return "walkRight";
  // A gesture must not hide a task that needs the user's attention.
  if (status === "failed") return "failed";
  if (status === "waiting-approval") return "waiting";
  if (interaction === "pet" || status === "done") return "celebrate";
  if (interaction === "hover") return "wave";
  if (status === "running") return "running";
  if (status === "thinking") return "thinking";
  return "idle";
}

export function taskStatusForPet(tasks: readonly { status: string }[]): TaskStatus {
  // Approval/input and failures remain visible even while another task runs.
  if (tasks.some(task => ["waiting-approval", "waiting-input"].includes(task.status))) return "waiting-approval";
  if (tasks.some(task => task.status === "failed")) return "failed";
  if (tasks.some(task => task.status === "running")) return "running";
  if (tasks.some(task => task.status === "completed")) return "done";
  return "idle";
}

export function frameAt(action: PetAction, elapsedMs: number) {
  const animation = animations[action];
  const total = animation.durations.reduce((sum: number, duration) => sum + duration, 0);
  let remaining = Math.max(0, Number.isFinite(elapsedMs) ? elapsedMs : 0) % total;
  for (let column = 0; column < animation.durations.length; column++) {
    const duration = animation.durations[column];
    if (remaining < duration) return { row: animation.row, column, nextIn: duration - remaining };
    remaining -= duration;
  }
  return { row: animation.row, column: 0, nextIn: animation.durations[0] };
}

export function petSize(scale: number) {
  const width = Math.round(32 * Math.min(5, Math.max(1, Number.isFinite(scale) ? scale : 3)));
  return { width, height: Math.round(width * atlas.frameHeight / atlas.frameWidth) };
}
