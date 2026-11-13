import type { LaneMetrics } from "@future-os/git-graph";

/**
 * The lane picture's measurements, in pixels.
 *
 * Apart from the component that paints them so that file exports one component
 * and nothing else: Fast Refresh (and the lint rule that enforces it) needs a
 * module's exports to be either all components or all values, and the panel
 * that lays rows out shares the row height with these.
 */

/** A lane's width. Narrow enough that a six-lane history fits a side panel, */
export const LANE_PITCH = 14;
export const TURN_RADIUS = 6;
/** How much room a crossing opens around the lane it passes. */
export const CROSSING_GAP = 3;
/**
 * The commit's dot. A merge's is a ring, so `o` and `M` stay distinguishable.
 */
export const DOT_RADIUS = 3.75;
export const DOT_STROKE = 1.75;
/** Room left and right of the picture, so a dot on lane 0 is not clipped. */
export const GUTTER_PAD = LANE_PITCH / 2;
/** Stroke width of a thread or a connection. */
export const THREAD_WIDTH = 2;

export const LANE_METRICS: LaneMetrics = {
  pitch: LANE_PITCH,
  radius: TURN_RADIUS,
  gap: CROSSING_GAP,
};
