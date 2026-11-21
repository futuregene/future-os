/**
 * The colours a lane picture may use, in lane order.
 *
 * Owned here rather than in a theme because it is part of the drawing's
 * contract, not of a screen's styling: the decoder assigns `lane % length`, and
 * a reader who has learned that six lanes cycle has learned it on the phone, on
 * the desktop and in the terminal. The two platforms differ in the exact hex —
 * a light theme needs deeper colours than a dark one — but not in ordering, so
 * a lane keeps its meaning across appearances.
 */
export type GraphPalette = readonly string[];

/**
 * Six, because the picture stops at six lanes: the producers (the host's
 * `git_history` and the TUI's `git_panel`, one window for all three surfaces)
 * draw every line past the sixth *in* the sixth column, so a reader never sees
 * more than six — and two lanes that share a colour are then six columns apart,
 * far enough to read as two columns rather than as one line skipping a row.
 */
export const LIGHT_GRAPH_COLORS: GraphPalette = [
  "#2563eb", // accent
  "#0e7490", // iris
  "#a16207", // amber
  "#15803d", // green
  "#b91c1c", // red
  "#7e22ce", // purple
];

export const DARK_GRAPH_COLORS: GraphPalette = [
  "#60a5fa", // accent
  "#38bdf8", // iris
  "#fbbf24", // amber
  "#4ade80", // green
  "#f87171", // red
  "#c084fc", // purple
];
