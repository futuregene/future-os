/**
 * Pure, DOM-free helpers for desktop image zoom.
 *
 * Design constraints:
 * - Scale is clamped to a minimum that fits the image inside the container and
 *   a generous maximum for reading dense screenshots. No value below "fit" is
 *   allowed, so the image never shrinks smaller than the available space.
 * - Panning is bounded so the image can never be dragged completely offscreen.
 * - Zooming uses the cursor as the anchor point: the pixel under the pointer
 *   stays under the pointer as scale changes.
 */

export const MIN_SCALE = 1;
export const MAX_SCALE = 6;

export interface Size {
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface ZoomState {
  scale: number;
  x: number;
  y: number;
}

/** Compute the uniform scale that makes the image fit entirely inside the container. */
export function fitScale(container: Size, image: Size): number {
  if (!Number.isFinite(image.width) || !Number.isFinite(image.height) || image.width <= 0 || image.height <= 0) {
    return 1;
  }
  return Math.min(container.width / image.width, container.height / image.height, 1);
}

/** Clamp scale between MIN_SCALE and MAX_SCALE (inclusive). */
export function clampScale(scale: number): number {
  if (Number.isNaN(scale))
    return MIN_SCALE;
  if (scale === Number.POSITIVE_INFINITY)
    return MAX_SCALE;
  if (scale === Number.NEGATIVE_INFINITY)
    return MIN_SCALE;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

/**
 * Clamp translation so the scaled image never leaves the container entirely.
 * At scale 1 the translation is always 0. For a given scale, the image occupies
 * `imageSize * scale` pixels; if it is smaller than the container it must be
 * centered, so panning is locked to 0. If it is larger, the user can pan by at
 * most the overhang on each side.
 */
export function clampTranslation(value: number, scale: number, containerSize: number, imageSize: number): number {
  if (!Number.isFinite(value))
    return 0;
  const scaled = imageSize * scale;
  const overhang = Math.max(0, scaled - containerSize) / 2;
  const clamped = Math.min(overhang, Math.max(-overhang, value));
  // +0 keeps vitest's toBe(0) assertions happy even when value is -0.
  return clamped === 0 ? 0 : clamped;
}

/**
 * Zoom towards a cursor anchor point while keeping that point fixed in screen
 * space. Returns the new scale and translation.
 */
export function zoomAtAnchor(
  container: Size,
  image: Size,
  state: ZoomState,
  anchor: Point,
  desiredScale: number,
): ZoomState {
  const fit = fitScale(container, image);
  const oldScalePx = fit * state.scale;
  const newScale = clampScale(desiredScale);
  const newScalePx = fit * newScale;

  // Pixel offset from container centre to anchor, before zoom.
  const dx = anchor.x - container.width / 2;
  const dy = anchor.y - container.height / 2;

  // The same image point must remain under the cursor. Derive the translation
  // adjustment from the ratio of the two scales.
  const ratio = newScalePx / oldScalePx;
  const newX = dx * (1 - ratio) + state.x * ratio;
  const newY = dy * (1 - ratio) + state.y * ratio;

  return {
    scale: newScale,
    x: clampTranslation(newX, newScale, container.width, image.width * fit),
    y: clampTranslation(newY, newScale, container.height, image.height * fit),
  };
}

/** Reset zoom/pan to the fully-fitted state. */
export function resetZoom(): ZoomState {
  return { scale: MIN_SCALE, x: 0, y: 0 };
}
