import { containsPoint, type PetHitRect, type PetWindowPoint } from "../petHitTest";

export interface ImageGeometry {
  bounds: PetHitRect;
  width: number;
  height: number;
  columns?: number;
  rows?: number;
  column?: number;
  row?: number;
}

// Both atlas frames and object-fit:contain images use the same alpha coordinates.
export function imagePixelAt(point: PetWindowPoint, geometry: ImageGeometry) {
  const { bounds, width, height, columns = 1, rows = 1, column = 0, row = 0 } = geometry;
  if (!containsPoint(bounds, point) || width <= 0 || height <= 0) return null;
  const frameWidth = width / columns;
  const frameHeight = height / rows;
  const displayWidth = bounds.right - bounds.left;
  const displayHeight = bounds.bottom - bounds.top;
  if (displayWidth <= 0 || displayHeight <= 0) return null;
  if (columns > 1 || rows > 1) {
    // Atlas CSS stretches each axis to the rounded display size; unlike a photo,
    // it has no contain letterbox. Sampling uses those exact two scale factors.
    const x = (point.x - bounds.left) * frameWidth / displayWidth;
    const y = (point.y - bounds.top) * frameHeight / displayHeight;
    if (x >= frameWidth || y >= frameHeight) return null;
    return { x: Math.floor(column * frameWidth + x), y: Math.floor(row * frameHeight + y) };
  }
  const scale = Math.min((bounds.right - bounds.left) / frameWidth, (bounds.bottom - bounds.top) / frameHeight);
  if (scale <= 0) return null;
  const left = bounds.left + ((bounds.right - bounds.left) - frameWidth * scale) / 2;
  const top = bounds.top + ((bounds.bottom - bounds.top) - frameHeight * scale) / 2;
  const x = (point.x - left) / scale;
  const y = (point.y - top) / scale;
  if (x < 0 || y < 0 || x >= frameWidth || y >= frameHeight) return null;
  return { x: Math.floor(column * frameWidth + x), y: Math.floor(row * frameHeight + y) };
}

const alphaCache = new WeakMap<HTMLImageElement, { src: string; width: number; height: number; data: Uint8ClampedArray }>();

export function isPointOnOpaquePetImage(root: HTMLElement, point: PetWindowPoint): boolean {
  const rootBounds = root.getBoundingClientRect();
  for (const frame of root.querySelectorAll<HTMLElement>("[data-pet-frame]")) {
    const avatar = frame.closest('.pet-avatar')?.getBoundingClientRect();
    if (avatar && !containsPoint({ left: avatar.left - rootBounds.left, top: avatar.top - rootBounds.top, right: avatar.right - rootBounds.left, bottom: avatar.bottom - rootBounds.top }, point)) continue;
    const rect = frame.getBoundingClientRect();
    const bounds = { left: rect.left - rootBounds.left, top: rect.top - rootBounds.top, right: rect.right - rootBounds.left, bottom: rect.bottom - rootBounds.top };
    if (!containsPoint(bounds, point)) continue;
    const img = frame.querySelector("img");
    if (!img?.complete || !img.naturalWidth) continue;
    const pixel = imagePixelAt(point, {
      bounds, width: img.naturalWidth, height: img.naturalHeight,
      columns: Number(frame.dataset.columns) || 1, rows: Number(frame.dataset.rows) || 1,
      column: Number(frame.dataset.column) || 0, row: Number(frame.dataset.row) || 0,
    });
    if (!pixel) continue;
    let alpha = alphaCache.get(img);
    if (!alpha || alpha.src !== img.currentSrc || alpha.width !== img.naturalWidth || alpha.height !== img.naturalHeight) {
      const canvas = document.createElement("canvas");
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      try {
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) return true;
        ctx.drawImage(img, 0, 0);
        alpha = { src: img.currentSrc, width: canvas.width, height: canvas.height, data: ctx.getImageData(0, 0, canvas.width, canvas.height).data };
        alphaCache.set(img, alpha);
      } catch { return true; }
    }
    if (alpha.data[(pixel.y * alpha.width + pixel.x) * 4 + 3] > 12) return true;
  }
  return false;
}
