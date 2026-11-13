/**
 * The Git history's lane picture, as the app draws it.
 *
 * The geometry lives in `packages/git-graph` because the desktop draws the same
 * picture: the phone, the desktop window and the terminal all show one history
 * of one workspace, and three implementations of the lane algorithm would drift
 * into three subtly different drawings of it. This module re-exports the shared
 * decoder so the chat feature keeps one import, and adds nothing of its own.
 */
export {
  graphGeometry,
  laneCount,
  laneDrawing,
  patchLines,
  type GraphGeometry,
  type LaneDrawing,
  type LaneMetrics,
  type PatchLine,
} from "@future-os/git-graph";
