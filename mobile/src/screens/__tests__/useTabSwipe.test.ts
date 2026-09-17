import { Animated, PanResponder, type GestureResponderEvent, type PanResponderGestureState } from "react-native";
import {
  createTabSwipeController,
  resolveTabSwipe,
  shouldClaimTabSwipe,
  type PageDirection,
  type TabSwipeOptions,
} from "../useTabSwipe";

/** Resolves slide-out/settle animations in one step — the offset lands on its
 * target and the completion fires — so a release is observable in a single
 * assertion; the real timings are the device's business. */
const instant = (value: Animated.Value | Animated.ValueXY, config: Animated.TimingAnimationConfig) =>
  ({
    start: (callback?: (result: { finished: boolean }) => void) => {
      (value as Animated.Value).setValue(config.toValue as number);
      callback?.({ finished: true });
    },
    stop: () => {},
    reset: () => {},
  }) as Animated.CompositeAnimation;

const event = {} as GestureResponderEvent;
const gesture = (dx: number, vx = 0) => ({ dx, dy: 0, vx, vy: 0 }) as PanResponderGestureState;

/** The controller with its handlers in hand, as the zoom controller's test does. */
function swipeHarness(overrides: Partial<TabSwipeOptions> = {}) {
  const timing = jest.spyOn(Animated, "timing").mockImplementation(instant);
  const create = jest.spyOn(PanResponder, "create");
  const onSwitch = jest.fn();
  const hasPage = jest.fn((direction: PageDirection) => direction === "next");
  const controller = createTabSwipeController({ hasPage, onSwitch, enabled: true, ...overrides });
  const handlers = create.mock.calls[0]![0];
  create.mockRestore();
  // 300pt page: a slow drag commits at 22% of it (66pt).
  controller.measure(300);
  const release = (dx: number, vx = 0) => handlers.onPanResponderRelease!(event, gesture(dx, vx));
  const drag = (dx: number, vx = 0) => {
    handlers.onPanResponderGrant!(event, gesture(dx, vx));
    handlers.onPanResponderMove!(event, gesture(dx, vx));
  };
  return {
    controller,
    handlers,
    hasPage,
    onSwitch,
    timing,
    drag,
    release,
    offset: () => (controller.translateX as Animated.Value & { __getValue(): number }).__getValue(),
    slideTarget: () => timing.mock.calls.at(-1)![1].toValue,
    canClaim: (dx: number, dy: number) =>
      handlers.onMoveShouldSetPanResponder!(event, { dx, dy, vx: 0, vy: 0 } as PanResponderGestureState),
  };
}

afterEach(() => {
  jest.restoreAllMocks();
});

test("a drag is a page swipe only once it is clearly horizontal", () => {
  expect(shouldClaimTabSwipe(-30, 4)).toBe(true);
  expect(shouldClaimTabSwipe(30, -4)).toBe(true);
  // Shorter than the claim distance: a finger wobble, not a page.
  expect(shouldClaimTabSwipe(-8, 0)).toBe(false);
  // Steep enough to be the list scrolling under a diagonal thumb.
  expect(shouldClaimTabSwipe(-30, 20)).toBe(false);
  expect(shouldClaimTabSwipe(0, -40)).toBe(false);
});

test("a slow drag commits by distance and a flick by speed", () => {
  expect(resolveTabSwipe(-80, 0, 300)).toBe("next");
  expect(resolveTabSwipe(80, 0, 300)).toBe("prev");
  // 40pt is well short of the 66pt the page needs at this width.
  expect(resolveTabSwipe(-40, 0, 300)).toBeNull();
  // A flick of the same length is deliberate.
  expect(resolveTabSwipe(-20, -1.2, 300)).toBe("next");
  expect(resolveTabSwipe(-20, 1.2, 300)).toBe("next");
});

test("the page follows the finger, then slides out before switching", () => {
  const { drag, release, offset, onSwitch, slideTarget, timing } = swipeHarness();
  drag(-120);
  expect(offset()).toBe(-120);
  release(-120);
  expect(slideTarget()).toBe(-300);
  expect(offset()).toBe(-300);
  expect(onSwitch).toHaveBeenCalledWith("next");
  expect(timing.mock.calls.at(-1)![1]).toMatchObject({ duration: 160, useNativeDriver: true });
});

test("a drag that stops short settles back without switching", () => {
  const { drag, release, offset, onSwitch, slideTarget } = swipeHarness();
  drag(-40);
  expect(offset()).toBe(-40);
  release(-40);
  expect(slideTarget()).toBe(0);
  expect(onSwitch).not.toHaveBeenCalled();
});

test("a drag toward a page that does not exist is damped and never commits", () => {
  const { drag, release, offset, hasPage, onSwitch, slideTarget } = swipeHarness();
  hasPage.mockReturnValue(false);
  drag(150);
  expect(offset()).toBe(45);
  release(150);
  expect(slideTarget()).toBe(0);
  expect(onSwitch).not.toHaveBeenCalled();
});

test("a cancelled drag leaves no page half off screen", () => {
  const { drag, handlers, offset, onSwitch, slideTarget } = swipeHarness();
  drag(-120);
  handlers.onPanResponderTerminate!(event, gesture(-120));
  expect(slideTarget()).toBe(0);
  expect(offset()).toBe(0);
  expect(onSwitch).not.toHaveBeenCalled();
});

test("the gesture is claimed by capture too, and held once claimed", () => {
  const { handlers, canClaim, hasPage } = swipeHarness();
  expect(canClaim(-30, 2)).toBe(true);
  expect(
    handlers.onMoveShouldSetPanResponderCapture!(event, gesture(-30)),
  ).toBe(true);
  expect(handlers.onPanResponderTerminationRequest!(event, gesture(-30))).toBe(false);
  expect(hasPage).not.toHaveBeenCalled();
});

test("a disabled swipe never claims the drag", () => {
  const { canClaim } = swipeHarness({ enabled: false });
  expect(canClaim(-30, 2)).toBe(false);
});

test("re-configuring a live controller retargets the next drag", () => {
  const { controller, drag, release, onSwitch, hasPage } = swipeHarness();
  controller.configure({
    enabled: true,
    hasPage,
    onSwitch: direction => onSwitch(`configured:${direction}`),
  });
  drag(-120);
  release(-120);
  expect(onSwitch).toHaveBeenCalledWith("configured:next");
});
