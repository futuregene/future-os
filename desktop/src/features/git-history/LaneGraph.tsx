import type { GraphGeometry } from "@future-os/git-graph";
import { laneDrawing } from "@future-os/git-graph";
import {
  DOT_RADIUS,
  DOT_STROKE,
  LANE_METRICS,
  THREAD_WIDTH,
} from "./laneMetrics";

/**
 * One commit's lane picture, drawn as strokes.
 *
 * The same decoder the phone uses (`@future-os/git-graph`) hands this component
 * SVG path data, so the two front ends draw one history one way. Nothing here
 * paints a glyph: the host's box-drawing grid is a description of the picture,
 * and at any size a column of characters reads as broken marks rather than as
 * lines.
 *
 * A plain `<svg>` element rather than a component library: the desktop has no
 * SVG wrapper, the geometry is already plain numbers, and a browser renders
 * paths natively.
 */
export function LaneGraph({
  geometry,
  height,
  width,
  center,
  opacity,
}: {
  geometry: GraphGeometry | null;
  height: number;
  width: number;
  /**
   * The y the horizontals run on and the dot is drawn at; the row's middle when
   * it is a single line of text.
   */
  center?: number;
  /** Fade the picture on a hovering row, so the row's own text leads. */
  opacity?: number;
}) {
  if (!geometry) {
    return null;
  }
  const drawing = laneDrawing(geometry, LANE_METRICS, height, center);
  return (
    <svg
      aria-hidden="true"
      data-lane-graph=""
      focusable="false"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      style={{ display: "block", opacity, transition: "opacity 120ms" }}
    >
      {drawing.threads.map(thread => (
        <line
          // One thread per lane, so its x is the lane's identity within the row.
          key={`t${thread.x}`}
          x1={thread.x}
          y1={thread.y1}
          x2={thread.x}
          y2={thread.y2}
          stroke={thread.color}
          strokeWidth={THREAD_WIDTH}
          strokeLinecap="round"
        />
      ))}
      {drawing.runs.map(run => (
        <path
          // The path data is the connection: two runs in one row cannot have the
          // same stretch and the same two ends.
          key={run.path}
          d={run.path}
          stroke={run.color}
          strokeWidth={THREAD_WIDTH}
          strokeLinecap="round"
          fill="none"
        />
      ))}
      {drawing.dots.map(dot => (
        // A merge is a ring, a plain commit a disc — the same distinction the
        // TUI's `M` and `o` make, so a reader who knows one reads the other.
        <circle
          // A row has at most one commit, so there is nothing to tell apart.
          key="dot"
          cx={dot.x}
          cy={dot.y}
          r={dot.merged ? DOT_RADIUS + DOT_STROKE / 2 : DOT_RADIUS}
          fill={dot.merged ? "none" : dot.color}
          stroke={dot.color}
          strokeWidth={dot.merged ? DOT_STROKE : 0}
        />
      ))}
    </svg>
  );
}
