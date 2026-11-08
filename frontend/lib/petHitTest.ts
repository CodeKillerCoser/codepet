export interface PetWindowPoint {
  x: number;
  y: number;
}

export interface PetHitRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export function shouldIgnorePetWindowCursor(point: PetWindowPoint, hitRects: PetHitRect[]) {
  return !hitRects.some((rect) => containsPoint(rect, point));
}

export function containsPoint(rect: PetHitRect, point: PetWindowPoint) {
  return point.x >= rect.left && point.x <= rect.right && point.y >= rect.top && point.y <= rect.bottom;
}

export function rectFromElementBounds(elementBounds: Pick<DOMRect, "left" | "top" | "right" | "bottom">, rootBounds: Pick<DOMRect, "left" | "top">, padding = 0): PetHitRect {
  return {
    left: elementBounds.left - rootBounds.left - padding,
    top: elementBounds.top - rootBounds.top - padding,
    right: elementBounds.right - rootBounds.left + padding,
    bottom: elementBounds.bottom - rootBounds.top + padding,
  };
}
