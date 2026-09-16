import { Animated, PanResponder, type GestureResponderEvent, type PanResponderGestureState } from "react-native";
import { MAX_SCALE, MIN_SCALE, clampScale, clampTranslation, createZoomController, pinchDistance } from "../components/ZoomableImage";

function gestureHarness() {
  const create = jest.spyOn(PanResponder, "create");
  const zoom = createZoomController();
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
  return { zoom, handlers, gesture, event, values };
}

test("the second finger claims the preview and repeated pinches change the actual transform", () => {
  const { handlers, gesture, event, values } = gestureHarness();
  expect(handlers.onStartShouldSetPanResponder!(event([150, 400]), gesture)).toBe(false);
  const start = event([150, 400], [250, 400]);
  expect(handlers.onStartShouldSetPanResponder!(start, gesture)).toBe(true);
  expect(handlers.onStartShouldSetPanResponderCapture!(start, gesture)).toBe(true);
  handlers.onPanResponderGrant!(start, gesture);
  handlers.onPanResponderStart!(start, gesture);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  expect(values()).toEqual([0, 0, 2]);
  handlers.onPanResponderRelease!(event(), gesture);
  handlers.onPanResponderGrant!(start, gesture);
  handlers.onPanResponderMove!(event([100, 400], [300, 400]), gesture);
  expect(values()).toEqual([0, 0, 4]);
  handlers.onPanResponderMove!(event([0, 400], [400, 400]), gesture);
  expect(values()[2]).toBe(MAX_SCALE);
  handlers.onPanResponderMove!(event([199, 400], [201, 400]), gesture);
  expect(values()).toEqual([0, 0, MIN_SCALE]);
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
