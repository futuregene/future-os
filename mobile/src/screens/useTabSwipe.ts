import { useEffect, useState } from "react";
import { Animated, Easing, PanResponder, type PanResponderInstance } from "react-native";

/**
 * Horizontal paging between the two list tabs (workspaces ⇄ conversations),
 * built on RN's own PanResponder/Animated rather than a gesture library: the app
 * ships no gesture stack and a swipe does not justify adding one (ZoomableImage
 * uses the same pair for the preview's pinch).
 *
 * The list itself scrolls vertically, so the whole job is not stealing its
 * drags: a gesture is claimed only once it is clearly horizontal (see
 * {@link shouldClaimTabSwipe}), and a direction with no page behind it is never
 * claimed at all.
 */

/** Travel a drag needs before it can be read as a page swipe. */
const CLAIM_DISTANCE = 12;
/** ... and it must be this many times longer than it is tall, so a thumb that
 * drifts while scrolling the list never pages it. */
const CLAIM_SLOPE = 2;
/** A slow drag commits once it covers this fraction of the page width. */
const COMMIT_FRACTION = 0.22;
/** A flick faster than this (px/ms) commits however short it was. */
const COMMIT_VELOCITY = 0.5;
/** How much of the finger's travel a drag keeps when it points off the end of
 * the row of pages (workspaces further left, conversations further right). Never
 * commits: the damping is what tells the finger the list has an edge. */
const EDGE_RESISTANCE = 0.3;
/** Slide-out / settle duration. */
const SLIDE_MS = 160;

/** The page the finger asks for: `next` is the one drawn to its right. */
export type PageDirection = "next" | "prev";

/** Whether a live drag has become a page swipe rather than a list scroll. */
export function shouldClaimTabSwipe(dx: number, dy: number): boolean {
  return Math.abs(dx) >= CLAIM_DISTANCE && Math.abs(dx) > Math.abs(dy) * CLAIM_SLOPE;
}

/** The page a released drag asks for, or null when it never travelled far
 * enough to mean one. `pageWidth` only sets the slow-drag distance. */
export function resolveTabSwipe(dx: number, vx: number, pageWidth: number): PageDirection | null {
  const committed =
    Math.abs(vx) >= COMMIT_VELOCITY ||
    Math.abs(dx) >= Math.max(CLAIM_DISTANCE, pageWidth * COMMIT_FRACTION);
  if (!committed) return null;
  return dx < 0 ? "next" : "prev";
}

export interface TabSwipeOptions {
  /** Whether the page in this direction exists. */
  hasPage(direction: PageDirection): boolean;
  /** Called once the outgoing page has slid off screen. */
  onSwitch(direction: PageDirection): void;
  /** False while the toolbar shows something other than the tabs (searching or
   * multi-selecting): the tab bar is hidden then, so a swipe would silently
   * change mode instead of moving between two visible pages. */
  enabled: boolean;
}

export interface TabSwipeController {
  /** Stable offset to apply to the page as its transform. */
  translateX: Animated.Value;
  panHandlers: PanResponderInstance["panHandlers"];
  /** Record the page's laid-out width: the slide distance and the slow-drag
   * commit threshold. */
  measure(width: number): void;
  /** Retarget a controller whose handlers are already live (see useTabSwipe). */
  configure(options: TabSwipeOptions): void;
}

/**
 * The swipe behind one page: pan handlers to spread onto the page view, the
 * offset to translate it by, and the measurement that sizes its slide.
 *
 * Everything mutable lives in closure variables of this factory rather than in
 * refs or state: the handlers are created once and only ever run at gesture
 * time, so render never reads the live values.
 */
export function createTabSwipeController(initial: TabSwipeOptions): TabSwipeController {
  const translateX = new Animated.Value(0);
  let options = initial;
  let pageWidth = 0;

  const animate = (toValue: number, easing: (value: number) => number, onDone?: () => void) => {
    Animated.timing(translateX, {
      toValue,
      duration: SLIDE_MS,
      easing,
      useNativeDriver: true,
    }).start(({ finished }) => {
      // A superseded animation (a new gesture, an unmount) must not switch tabs.
      if (finished) onDone?.();
    });
  };

  const panHandlers = PanResponder.create({
    // Bubble and capture both ask, so the drag is claimed in either delivery
    // order; capturing is what cancels the row press it started on.
    onMoveShouldSetPanResponder: (_event, gesture) =>
      options.enabled && shouldClaimTabSwipe(gesture.dx, gesture.dy),
    onMoveShouldSetPanResponderCapture: (_event, gesture) =>
      options.enabled && shouldClaimTabSwipe(gesture.dx, gesture.dy),
    // A settle still running from the previous swipe would otherwise fight the
    // finger for the offset.
    onPanResponderGrant: () => translateX.stopAnimation(),
    // Once claimed, the page owns the gesture: handing it to the list mid-drag
    // would leave the page parked half off screen.
    onPanResponderTerminationRequest: () => false,
    onPanResponderMove: (_event, gesture) => {
      const direction: PageDirection = gesture.dx < 0 ? "next" : "prev";
      translateX.setValue(options.hasPage(direction) ? gesture.dx : gesture.dx * EDGE_RESISTANCE);
    },
    onPanResponderRelease: (_event, gesture) => {
      const direction = resolveTabSwipe(gesture.dx, gesture.vx, pageWidth);
      if (direction && options.hasPage(direction)) {
        // Switch only after the page is off screen, so the incoming one is
        // already at rest rather than appearing under a half-slid list.
        animate(
          direction === "next" ? -pageWidth : pageWidth,
          Easing.in(Easing.cubic),
          () => options.onSwitch(direction),
        );
        return;
      }
      animate(0, Easing.out(Easing.cubic));
    },
    onPanResponderTerminate: () => animate(0, Easing.out(Easing.cubic)),
  }).panHandlers;

  return {
    translateX,
    panHandlers,
    measure: width => {
      pageWidth = width;
    },
    configure: next => {
      options = next;
    },
  };
}

/**
 * Create the page swipe for one list. The controller is built once — rebuilding
 * the handlers mid-drag would drop the gesture — and re-configured with the
 * latest props and mode on every render instead.
 */
export function useTabSwipe(options: TabSwipeOptions): TabSwipeController {
  const [controller] = useState(() => createTabSwipeController(options));
  useEffect(() => {
    controller.configure(options);
  });
  return controller;
}
