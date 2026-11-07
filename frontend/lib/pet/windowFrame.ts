type Point = { x: number; y: number };
type Size = { width: number; height: number };

/** All values are physical pixels. Preserve the pet's bottom-right screen anchor. */
export function bottomAnchoredPosition(position: Point, previous: Size, next: Size): Point {
  return {
    x: Math.round(position.x + previous.width - next.width),
    y: Math.round(position.y + previous.height - next.height),
  };
}
