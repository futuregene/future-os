import { MAX_SCALE, MIN_SCALE, clampScale, clampTranslation, pinchDistance } from "../components/ZoomableImage";

// The pan limits are what keep a zoomed picture from being dragged out of sight,
// so they carry the safety of the interaction and are pinned here directly.

test("a pinch is measured as the distance between the two touches", () => {
  expect(pinchDistance([{ pageX: 0, pageY: 0 }, { pageX: 3, pageY: 4 }])).toBe(5);
  // Fewer than two fingers down is not a pinch — the caller falls back to panning.
  expect(pinchDistance([{ pageX: 0, pageY: 0 }])).toBeNull();
  expect(pinchDistance([])).toBeNull();
});

test("zoom stays inside its bounds", () => {
  expect(clampScale(1)).toBe(1);
  expect(clampScale(3.5)).toBe(3.5);
  // A pinch can multiply past the ceiling; a deep pinch cannot zoom below fit.
  expect(clampScale(99)).toBe(MAX_SCALE);
  expect(clampScale(0.2)).toBe(MIN_SCALE);
  expect(clampScale(-4)).toBe(MIN_SCALE);
  expect(clampScale(Number.NaN)).toBe(MIN_SCALE);
});

test("panning is limited to the overhang the zoom created", () => {
  // At fit there is nothing to pan: the picture exactly fills the frame.
  expect(clampTranslation(50, 1, 400)).toBe(0);
  // At 2× the content sticks out half a frame on each side.
  expect(clampTranslation(50, 2, 400)).toBe(50);
  expect(clampTranslation(999, 2, 400)).toBe(200);
  expect(clampTranslation(-999, 2, 400)).toBe(-200);
  // At 6× the overhang is 2.5 frames, so a drag inside that range is followed
  // and one beyond it pins at the edge.
  expect(clampTranslation(999, 6, 400)).toBe(999);
  expect(clampTranslation(5000, 6, 400)).toBe(1000);
  expect(clampTranslation(Number.NaN, 3, 400)).toBe(0);
});
