import { Animated, PanResponder, type GestureResponderEvent, type PanResponderGestureState } from "react-native";
import {
  DOUBLE_TAP_MS,
  DOUBLE_TAP_SCALE,
  MAX_SCALE,
  MIN_SCALE,
  clampScale,
  clampTranslation,
  createZoomController,
  pinchDistance,
} from "../components/ZoomableImage";

function gestureHarness(options: { now?: () => number } = {}) {
  const create = jest.spyOn(PanResponder, "create");
  const zoom = createZoomController(options);
  const handlers = create.mock.calls[0]![0];
  create.mockRestore();
  zoom.measure(400, 600, 0, 100);
  const gesture = {} as PanResponderGestureState;
  const event = (...points: [number, number][]) => ({
    nativeEvent: { touches: points.map(([pageX, pageY]) => ({ pageX, pageY })) },
  }) as GestureResponderEvent;
  const values = () => zoom.transform.map(track =>
    (Object.values(track)[0] as Animated.Value & { __getValue(): number }).__getValue(),
  );
  /** A press that starts and ends at one point, the way a tap arrives. */
  const tapAt = (x: number, y: number) => {
    const start = event([x, y]);
    handlers.onPanResponderGrant!(start, { x0: x, y0: y, dx: 0, dy: 0 } as PanResponderGestureState);
    handlers.onPanResponderRelease!(start, { x0: x, y0: y, dx: 0, dy: 0 } as PanResponderGestureState);
  };
  return { zoom, handlers, gesture, event, values, tapAt };
}

test("the picture owns the gesture from the first finger, and repeated pinches change the transform", () => {
  const { handlers, gesture, event, values } = gestureHarness();
  // Claiming only on the second finger would leave the zoom dependent on how the
  // surrounding container handles the initial touch, which is what broke on device.
  expect(handlers.onStartShouldSetPanResponder!(event([150, 400]), gesture)).toBe(true);
  expect(handlers.onStartShouldSetPanResponderCapture!(event([150, 400]), gesture)).toBe(true);
  expect(handlers.onMoveShouldSetPanResponder!(event([150, 400]), gesture)).toBe(true);
  const start = event([150, 400], [250, 400]);
  handlers.onPanResponderGrant!(start, { x0: 150, y0: 400, dx: 0, dy: 0 } as PanResponderGestureState);
  handlers.onPanResponderStart!(start, gesture);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  expect(values()).toEqual([0, 0, 2]);
  handlers.onPanResponderRelease!(event(), gesture);
  handlers.onPanResponderGrant!(start, { x0: 150, y0: 400, dx: 0, dy: 0 } as PanResponderGestureState);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  expect(values()).toEqual([0, 0, 4]);
  handlers.onPanResponderMove!(event([0, 400], [400, 400]), gesture);
  expect(values()[2]).toBe(MAX_SCALE);
  handlers.onPanResponderMove!(event([199, 400], [201, 400]), gesture);
  expect(values()).toEqual([0, 0, MIN_SCALE]);
});

test("a double-tap zooms in about the tapped point and a third tap returns to fit", () => {
  const now = jest.fn(() => 1_000);
  const { zoom, values, tapAt } = gestureHarness({ now });
  // A lone tap leaves the picture alone.
  tapAt(300, 500);
  expect(values()).toEqual([0, 0, MIN_SCALE]);
  // The second tap inside the window zooms to the tapped point. The double-tap is
  // delivered by a spring, so read the controller's state rather than the
  // animated value the spring is driving. The tap sits 100px right/below the frame
  // centre (300,500 vs 200,400), so at 2.5× holding it there needs −150,−150.
  now.mockReturnValue(1_000 + DOUBLE_TAP_MS);
  tapAt(300, 500);
  expect(zoom.state()).toEqual({ scale: DOUBLE_TAP_SCALE, x: -150, y: -150 });
  // And the next pair returns to fit.
  now.mockReturnValue(2_000);
  tapAt(300, 500);
  now.mockReturnValue(2_000 + DOUBLE_TAP_MS);
  tapAt(300, 500);
  expect(zoom.state()).toEqual({ scale: MIN_SCALE, x: 0, y: 0 });
});

test("two slow, far-apart or travelling presses are not a double-tap", () => {
  const now = jest.fn(() => 1_000);
  const { zoom, tapAt, handlers, event } = gestureHarness({ now });
  // Too slow: outside the window.
  tapAt(300, 500);
  now.mockReturnValue(1_000 + DOUBLE_TAP_MS + 1);
  tapAt(300, 500);
  expect(zoom.state().scale).toBe(MIN_SCALE);
  // In time but on a different spot.
  now.mockReturnValue(3_000);
  tapAt(60, 120);
  now.mockReturnValue(3_050);
  tapAt(300, 500);
  expect(zoom.state().scale).toBe(MIN_SCALE);
  // A drag is a pan attempt, never a tap: two quick press-and-drags must not zoom.
  now.mockReturnValue(4_000);
  const drag = event([300, 500]);
  handlers.onPanResponderGrant!(drag, { x0: 300, y0: 500, dx: 0, dy: 0 } as PanResponderGestureState);
  handlers.onPanResponderRelease!(drag, { x0: 300, y0: 500, dx: 90, dy: 0 } as PanResponderGestureState);
  now.mockReturnValue(4_050);
  handlers.onPanResponderGrant!(drag, { x0: 300, y0: 500, dx: 0, dy: 0 } as PanResponderGestureState);
  handlers.onPanResponderRelease!(drag, { x0: 300, y0: 500, dx: 90, dy: 0 } as PanResponderGestureState);
  expect(zoom.state().scale).toBe(MIN_SCALE);
});

test("a two-finger press is never read as a tap", () => {
  const now = jest.fn(() => 1_000);
  const { zoom, handlers, event } = gestureHarness({ now });
  for (const at of [1_000, 1_020]) {
    now.mockReturnValue(at);
    const both = event([290, 500], [310, 500]);
    handlers.onPanResponderGrant!(both, { x0: 300, y0: 500, dx: 0, dy: 0 } as PanResponderGestureState);
    handlers.onPanResponderRelease!(both, { x0: 300, y0: 500, dx: 0, dy: 0 } as PanResponderGestureState);
  }
  expect(zoom.state().scale).toBe(MIN_SCALE);
});

test("an off-center pinch keeps its focal point and follows midpoint movement", () => {
  const { handlers, gesture, event, values } = gestureHarness();
  handlers.onPanResponderGrant!(event([250, 450], [350, 450]), gesture);
  // Relative to the frame center (200, 400), the focal point is (100, 50).
  handlers.onPanResponderMove!(event([200, 450], [400, 450]), gesture);
  expect(values()).toEqual([-100, -50, 2]);
  handlers.onPanResponderMove!(event([220, 480], [420, 480]), gesture);
  expect(values()).toEqual([-80, -20, 2]);
});

test("lifting and adding a finger rebases pan/pinch without jumping to the original drag", () => {
  const { handlers, gesture, event, values } = gestureHarness();
  handlers.onPanResponderGrant!(event([150, 400], [250, 400]), gesture);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  expect(values()).toEqual([0, 0, 2]);
  handlers.onPanResponderEnd!(event([300, 400]), gesture);
  handlers.onPanResponderMove!(event([300, 400]), gesture);
  expect(values()).toEqual([0, 0, 2]);
  handlers.onPanResponderMove!(event([320, 430]), gesture);
  expect(values()).toEqual([20, 30, 2]);
  handlers.onPanResponderStart!(event([120, 430], [320, 430]), gesture);
  handlers.onPanResponderMove!(event([120, 430], [320, 430]), gesture);
  expect(values()).toEqual([20, 30, 2]);
  handlers.onPanResponderMove!(event([20, 430], [420, 430]), gesture);
  expect(values()).toEqual([20, 30, 4]);
  expect(handlers.onPanResponderTerminationRequest!(event(), gesture)).toBe(false);
});

test("resizing resets the preview to fit while position-only measurement preserves zoom", () => {
  const { zoom, handlers, gesture, event, values } = gestureHarness();
  handlers.onPanResponderGrant!(event([150, 400], [250, 400]), gesture);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  zoom.measure(400, 600, 0, 120);
  expect(values()[2]).toBe(2);
  zoom.measure(600, 400);
  expect(values()).toEqual([0, 0, 1]);
});

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
