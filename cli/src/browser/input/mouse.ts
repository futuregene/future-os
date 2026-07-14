/**
 * Mouse helper — click coordinate calculations.
 */
export interface ElementBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Calculate the center of an element's bounding box. */
export function centerOf(box: ElementBox): { x: number; y: number } {
  return {
    x: Math.round(box.x + box.width / 2),
    y: Math.round(box.y + box.height / 2),
  };
}
