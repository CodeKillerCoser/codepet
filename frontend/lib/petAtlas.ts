import type { TaskStatus } from "./types";

export interface AtlasFrame {
  column: number;
  durationMs: number;
}

export interface AtlasAnimation {
  row: number;
  frames: AtlasFrame[];
}

const animations = {
  idle: frameRow(0, [280, 110, 110, 140, 140, 320]),
  failed: frameRow(5, [140, 140, 140, 140, 140, 140, 140, 240]),
  waiting: frameRow(6, [150, 150, 150, 150, 150, 260]),
  running: frameRow(7, [120, 120, 120, 120, 120, 220]),
  review: frameRow(8, [150, 150, 150, 150, 150, 280]),
} satisfies Record<string, AtlasAnimation>;

export function atlasAnimationForStatus(status: TaskStatus): AtlasAnimation {
  switch (status) {
    case "thinking":
      return animations.review;
    case "running":
      return animations.running;
    case "waiting-approval":
      return animations.waiting;
    case "failed":
      return animations.failed;
    case "idle":
    case "done":
    default:
      return animations.idle;
  }
}

function frameRow(row: number, durations: number[]): AtlasAnimation {
  return {
    row,
    frames: durations.map((durationMs, column) => ({ column, durationMs })),
  };
}
