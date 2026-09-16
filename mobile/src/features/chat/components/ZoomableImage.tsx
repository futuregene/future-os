import { useState } from "react";
import { Animated, PanResponder, StyleSheet, View } from "react-native";
import { colors } from "../../../theme/tokens";

/** Zoom bounds. 1 is "fit the frame"; the ceiling is generous enough to read a
 * dense screenshot on a phone without letting a stray pinch fly off to nothing. */
export const MIN_SCALE = 1;
export const MAX_SCALE = 6;

/** Below this, a released pinch springs back to fit — an easy way to reset. */
const RESET_THRESHOLD = 1.05;

type Touch = { pageX: number; pageY: number };

/** Distance between the first two touches, or null when fewer than two are down. */
export function pinchDistance(touches: Touch[]): number | null {
  const [a, b] = touches;
  if (!a || !b) return null;
  return Math.hypot(a.pageX - b.pageX, a.pageY - b.pageY);
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
  measure(width: number, height: number, pageX?: number, pageY?: number): void;
}

/**
 * Build the pinch/pan controller. Everything mutable lives in closure variables of
 * this factory rather than in refs or state: the handlers are created once and
 * only ever run at gesture time, so render never inspects the live values, and
 * React's rules allow neither ref reads nor state mutation during render.
 *
 * Both the bounds and responder transitions are covered by tests.
 */
export function createZoomController(): ZoomController {
  const scale = new Animated.Value(MIN_SCALE);
  const translateX = new Animated.Value(0);
  const translateY = new Animated.Value(0);
  let current: GestureState = { scale: MIN_SCALE, x: 0, y: 0 };
  let start: GestureStart = { scale: MIN_SCALE, x: 0, y: 0, distance: 0, centerX: 0, centerY: 0 };
  const frame = { width: 0, height: 0, pageX: 0, pageY: 0 };

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
   * Zoom about the pinch midpoint. Scaling by (start × distance ratio) rather than
   * multiplying per-frame keeps the gesture free of compounding error and makes
   * the back-to-fit release below trivially reversible.
   */
  const zoom = (distance: number, [centerX, centerY]: [number, number]) => {
    const next = clampScale(start.scale * (distance / start.distance));
    // Keep the point under the fingers pinned while the content grows around it.
    const ratio = next / start.scale;
    const anchorX = start.centerX - frame.pageX - frame.width / 2;
    const anchorY = start.centerY - frame.pageY - frame.height / 2;
    const nextX = clampTranslation(centerX - start.centerX + anchorX * (1 - ratio) + start.x * ratio, next, frame.width);
    const nextY = clampTranslation(centerY - start.centerY + anchorY * (1 - ratio) + start.y * ratio, next, frame.height);
    current = { scale: next, x: nextX, y: nextY };
    scale.setValue(next);
    translateX.setValue(nextX);
    translateY.setValue(nextY);
  };

  const begin = (touches: Touch[]) => {
    const [centerX, centerY] = centerOf(touches);
    start = { ...current, distance: pinchDistance(touches) ?? 0, centerX, centerY };
  };

  const responder = PanResponder.create({
    // Claim the second finger immediately, including when it lands on the image.
    // Waiting for movement discards the initial part of the pinch.
    onStartShouldSetPanResponder: event => event.nativeEvent.touches.length === 2,
    onStartShouldSetPanResponderCapture: event => event.nativeEvent.touches.length === 2,
    // Claim the gesture once it is unambiguously a pinch (two fingers) or a drag
    // while already zoomed in.
    onMoveShouldSetPanResponder: (event, gesture) =>
      event.nativeEvent.touches.length === 2
      || (current.scale > MIN_SCALE && (Math.abs(gesture.dx) > 2 || Math.abs(gesture.dy) > 2)),
    onPanResponderGrant: event => {
      scale.stopAnimation();
      translateX.stopAnimation();
      translateY.stopAnimation();
      begin(event.nativeEvent.touches as Touch[]);
    },
    onPanResponderStart: event => begin(event.nativeEvent.touches as Touch[]),
    onPanResponderEnd: event => begin(event.nativeEvent.touches as Touch[]),
    onPanResponderTerminationRequest: () => false,
    onPanResponderMove: event => {
      const touches = event.nativeEvent.touches as Touch[];
      const distance = pinchDistance(touches);
      if (distance !== null) {
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
    onPanResponderRelease: () => {
      // Back to fit when barely zoomed, so no separate reset control is needed.
      if (current.scale < RESET_THRESHOLD) settle({ scale: MIN_SCALE, x: 0, y: 0 });
      else settle(current);
    },
    onPanResponderTerminate: () => settle(current),
  });

  return {
    panHandlers: responder.panHandlers,
    transform: [{ translateX }, { translateY }, { scale }],
    measure(width, height, pageX = 0, pageY = 0) {
      const resized = frame.width !== width || frame.height !== height;
      Object.assign(frame, { width, height, pageX, pageY });
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
  if (!b) return [a.pageX, a.pageY];
  return [(a.pageX + b.pageX) / 2, (a.pageY + b.pageY) / 2];
}

/**
 * A pinch-to-zoom, drag-to-pan image for the file preview. Built on RN's own
 * PanResponder/Animated rather than a gesture library: the app ships no gesture
 * stack, and adding one (plus its native build config) for a single preview
 * surface is a large dependency for one interaction.
 *
 * Bounds and finger transitions are tested through the controller; native gesture
 * delivery still needs device validation.
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
        // Touches use screen coordinates; account for the modal header/safe area.
        event.currentTarget.measureInWindow((x, y) => zoom.measure(width, height, x, y));
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
