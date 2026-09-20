import { useState } from "react";
import { Animated, PanResponder, StyleSheet, View } from "react-native";
import { colors } from "../../../theme/tokens";

/** Zoom bounds. 1 is "fit the frame"; the ceiling is generous enough to read a
 * dense screenshot on a phone without letting a stray pinch fly off to nothing. */
export const MIN_SCALE = 1;
export const MAX_SCALE = 6;

/** Below this, a released pinch springs back to fit — an easy way to reset. */
const RESET_THRESHOLD = 1.05;

/** Double-tap zoom target, and the window/slop that make two taps a double-tap. */
export const DOUBLE_TAP_SCALE = 2.5;
export const DOUBLE_TAP_MS = 300;
const TAP_SLOP = 14;

type Touch = { locationX: number; locationY: number };

/** Distance between the first two touches, or null when fewer than two are down. */
export function pinchDistance(touches: Touch[]): number | null {
  const [a, b] = touches;
  if (!a || !b) return null;
  return Math.hypot(a.locationX - b.locationX, a.locationY - b.locationY);
}

/** Hold a zoom factor inside {@link MIN_SCALE}..{@link MAX_SCALE}. */
export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) return MIN_SCALE;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

/**
 * Limit panning to the overhang the current zoom actually created: at scale `s`
 * the content sticks out `(s - 1) / 2` frames on each side, so there is never a
 * way to drag the picture off screen and lose it.
 */
export function clampTranslation(value: number, scale: number, frameSize: number): number {
  if (!Number.isFinite(value)) return 0;
  const limit = Math.max(0, ((scale - 1) * frameSize) / 2);
  return Math.min(limit, Math.max(-limit, value));
}

/** Mutable bookkeeping for one preview: where the content is, where the in-flight
 * gesture began, and how big the frame is (which bounds panning). */
type GestureState = { scale: number; x: number; y: number };
type GestureStart = GestureState & { distance: number; centerX: number; centerY: number };

/**
 * The zoom controller for one preview surface: the pan handlers to spread onto a
 * view, the transform to apply to the image, and the frame measurement that bounds
 * panning.
 */
export interface ZoomController {
  panHandlers: ReturnType<typeof PanResponder.create>["panHandlers"];
  /** One transform entry per track, in the order RN applies them. */
  transform: (
    | { translateX: Animated.Value }
    | { translateY: Animated.Value }
    | { scale: Animated.Value }
  )[];
  measure(width: number, height: number): void;
  /**
   * The zoom/pan the controller currently holds. Render never reads it; it exists
   * so a caller can observe where a gesture ended up, including the changes a
   * spring delivers rather than a direct `setValue` (a double-tap).
   */
  state(): { scale: number; x: number; y: number };
}

/** Injectable clock, so the double-tap window is testable without fake timers. */
export interface ZoomOptions {
  now?: () => number;
}

/**
 * Build the pinch/pan controller. Everything mutable lives in closure variables of
 * this factory rather than in refs or state: the handlers are created once and
 * only ever run at gesture time, so render never inspects the live values, and
 * React's rules allow neither ref reads nor state mutation during render.
 *
 * Distances and focal points come from the touches' `locationX`/`locationY` — the
 * coordinates relative to this view — never from `pageX`/`pageY`. Page
 * coordinates would have to be compared against a measured window origin, and
 * that origin is measured once per layout: on a sheet that slides in, the layout
 * lands while the surface is still below the viewport, so the stored origin is
 * off by most of a screen and every pinch then drags the picture away from the
 * fingers. Relative coordinates are recomputed from each event and cannot go
 * stale.
 *
 * Both the bounds and responder transitions are covered by tests.
 */
export function createZoomController({ now = Date.now }: ZoomOptions = {}): ZoomController {
  const scale = new Animated.Value(MIN_SCALE);
  const translateX = new Animated.Value(0);
  const translateY = new Animated.Value(0);
  let current: GestureState = { scale: MIN_SCALE, x: 0, y: 0 };
  let start: GestureStart = { scale: MIN_SCALE, x: 0, y: 0, distance: 0, centerX: 0, centerY: 0 };
  let lastTap: { at: number; x: number; y: number } | null = null;
  // Set for the duration of one press: where it started (relative to this view),
  // and whether a second finger was ever down. Release turns these into
  // tap-vs-pinch/pan.
  let press: { x: number; y: number } | null = null;
  let multiFinger = false;
  const frame = { width: 0, height: 0 };

  const settle = (next: GestureState) => {
    current = next;
    Animated.parallel([
      Animated.spring(scale, { toValue: next.scale, useNativeDriver: true, bounciness: 0 }),
      Animated.spring(translateX, { toValue: next.x, useNativeDriver: true, bounciness: 0 }),
      Animated.spring(translateY, { toValue: next.y, useNativeDriver: true, bounciness: 0 }),
    ]).start();
  };

  /** Pan the (already zoomed) picture, clamped to its actual overhang. */
  const pan = (dx: number, dy: number) => {
    const nextX = clampTranslation(start.x + dx, current.scale, frame.width);
    const nextY = clampTranslation(start.y + dy, current.scale, frame.height);
    current = { ...current, x: nextX, y: nextY };
    translateX.setValue(nextX);
    translateY.setValue(nextY);
  };

  /**
   * Focal-point zoom: grow the content about (`centerX`, `centerY`) so that point
   * stays under the fingers, then clamp the result. Shared by the pinch (which
   * scales by a distance ratio, so the gesture accumulates no compounding error)
   * and the double-tap (which moves to an absolute scale).
   */
  const zoomTo = (
    base: GestureStart,
    next: number,
    [centerX, centerY]: [number, number],
  ): GestureState => {
    const ratio = next / base.scale;
    // The view's own centre is the transform origin, and the touches are already
    // relative to the view, so no window origin enters this.
    const anchorX = base.centerX - frame.width / 2;
    const anchorY = base.centerY - frame.height / 2;
    const nextX = clampTranslation(
      centerX - base.centerX + anchorX * (1 - ratio) + base.x * ratio, next, frame.width);
    const nextY = clampTranslation(
      centerY - base.centerY + anchorY * (1 - ratio) + base.y * ratio, next, frame.height);
    return { scale: next, x: nextX, y: nextY };
  };

  const zoom = (distance: number, center: [number, number]) => {
    const next = clampScale(start.scale * (distance / start.distance));
    const state = zoomTo(start, next, center);
    current = state;
    scale.setValue(state.scale);
    translateX.setValue(state.x);
    translateY.setValue(state.y);
  };

  const begin = (touches: Touch[]) => {
    const [centerX, centerY] = centerOf(touches);
    start = { ...current, distance: pinchDistance(touches) ?? 0, centerX, centerY };
  };

  /**
   * A tap on the picture, in coordinates relative to this view: the second tap in
   * quick succession zooms in about that point, a third returns to fit. A single
   * finger is the only way to zoom when two-finger gestures are unavailable
   * (assistive settings, a stolen gesture) or simply not obvious, so the surface
   * must not depend on pinch alone.
   */
  const tap = (x: number, y: number) => {
    const at = now();
    const previous = lastTap;
    const isDouble = previous !== null
      && at - previous.at <= DOUBLE_TAP_MS
      && Math.abs(x - previous.x) <= TAP_SLOP
      && Math.abs(y - previous.y) <= TAP_SLOP;
    lastTap = isDouble ? null : { at, x, y };
    if (!isDouble) return;
    const next = current.scale > MIN_SCALE ? MIN_SCALE : DOUBLE_TAP_SCALE;
    settle(zoomTo(
      { ...current, distance: 0, centerX: x, centerY: y },
      clampScale(next),
      [x, y],
    ));
  };

  const responder = PanResponder.create({
    // Claim the surface on the FIRST finger, at the capture phase, so the gesture
    // is owned from the outset: waiting for a second finger to negotiate leaves
    // the zoom at the mercy of whatever the surrounding container does with the
    // initial touch. The picture has no other gesture to protect, and a press that
    // does not travel is only read as a double-tap candidate (see `tap`).
    onStartShouldSetPanResponder: () => true,
    onStartShouldSetPanResponderCapture: () => true,
    onMoveShouldSetPanResponder: () => true,
    onPanResponderGrant: (event, gesture) => {
      scale.stopAnimation();
      translateX.stopAnimation();
      translateY.stopAnimation();
      const touches = event.nativeEvent.touches as Touch[];
      begin(touches);
      // Where this press started, relative to this view, and whether two fingers
      // were ever down: together with the release travel this decides
      // tap-vs-pinch/pan. `gesture.x0/y0` would be page coordinates, which must
      // not be mixed with the relative ones the rest of the gesture uses.
      const origin = touches[0] ?? event.nativeEvent;
      press = { x: origin.locationX, y: origin.locationY };
      multiFinger = touches.length > 1;
    },
    onPanResponderStart: event => begin(event.nativeEvent.touches as Touch[]),
    onPanResponderEnd: event => begin(event.nativeEvent.touches as Touch[]),
    onPanResponderTerminationRequest: () => false,
    onPanResponderMove: event => {
      const touches = event.nativeEvent.touches as Touch[];
      const distance = pinchDistance(touches);
      if (distance !== null) {
        multiFinger = true;
        // A finger landed or lifted mid-gesture: restart from the live state
        // rather than jumping by a ratio against a stale distance.
        if (start.distance === 0) begin(touches);
        else zoom(distance, centerOf(touches));
        return;
      }
      if (start.distance !== 0) begin(touches);
      if (current.scale > MIN_SCALE && touches.length === 1) {
        const [centerX, centerY] = centerOf(touches);
        pan(centerX - start.centerX, centerY - start.centerY);
      }
    },
    onPanResponderRelease: (_event, gesture) => {
      const started = press;
      press = null;
      if (started && !multiFinger
        && Math.abs(gesture.dx) <= TAP_SLOP && Math.abs(gesture.dy) <= TAP_SLOP) {
        // A tap settles only when it turns out to be a double-tap; a lone tap
        // leaves the picture as it was.
        tap(started.x, started.y);
        return;
      }
      // Back to fit when barely zoomed, so no separate reset control is needed.
      if (current.scale < RESET_THRESHOLD) settle({ scale: MIN_SCALE, x: 0, y: 0 });
      else settle(current);
    },
    onPanResponderTerminate: () => settle(current),
  });

  return {
    panHandlers: responder.panHandlers,
    transform: [{ translateX }, { translateY }, { scale }],
    state: () => ({ ...current }),
    measure(width, height) {
      const resized = frame.width !== width || frame.height !== height;
      Object.assign(frame, { width, height });
      if (resized) {
        // A new frame has different pan bounds; do not animate through the old ones.
        current = { scale: MIN_SCALE, x: 0, y: 0 };
        scale.setValue(MIN_SCALE);
        translateX.setValue(0);
        translateY.setValue(0);
      }
    },
  };
}

function centerOf(touches: Touch[]): [number, number] {
  const [a, b] = touches;
  if (!a) return [0, 0];
  if (!b) return [a.locationX, a.locationY];
  return [(a.locationX + b.locationX) / 2, (a.locationY + b.locationY) / 2];
}

/**
 * A pinch-to-zoom, drag-to-pan image for the file preview. Built on RN's own
 * PanResponder/Animated rather than a gesture library: the app ships no gesture
 * stack, and adding one (plus its native build config) for a single preview
 * surface is a large dependency for one interaction.
 *
 * Bounds and finger transitions are covered by tests through the controller; the
 * gesture itself is verified in the harness by dispatching real touch sequences.
 */
export function ZoomableImage({ uri, accessibilityLabel }: {
  uri: string;
  accessibilityLabel?: string;
}) {
  const [zoom] = useState(() => createZoomController());

  return (
    <View
      accessibilityLabel={accessibilityLabel}
      onLayout={event => {
        const { width, height } = event.nativeEvent.layout;
        zoom.measure(width, height);
      }}
      style={styles.frame}
      {...zoom.panHandlers}
    >
      <View pointerEvents="none" style={styles.image}>
        <Animated.Image
          resizeMode="contain"
          source={{ uri }}
          style={[styles.image, { transform: zoom.transform }]}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  frame: { flex: 1, width: "100%", backgroundColor: colors.surfaceSubtle, overflow: "hidden" },
  image: { flex: 1, width: "100%", height: "100%" },
});
