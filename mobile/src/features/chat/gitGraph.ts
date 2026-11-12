import type { GraphPalette } from "../../theme/tokens";

/** A row's gutter, as the host writes it (`host/src/remote_host/git_graph.rs`,
 * the TUI's vocabulary), is a *description* of the drawing rather than the
 * drawing: each glyph says which directions have a line at that cell, and the
 * panel turns that into strokes. Nothing here paints characters. */
const COMMIT = new Set(["o", "M"]);

/** Which directions each lane glyph carries a *vertical* in.
 *
 * Only up/down are read from a glyph. Left and right are not: the gutter spells
 * horizontals out in the odd cells (`─` at `2n+1` joins lanes n and n+1), which
 * is unambiguous, while the junction glyphs are not — `┴` is both a corner
 * (up + right) and a T-junction (up + left + right), and the host emits the same
 * character for both. Reading horizontals from the `─` cells instead recovers
 * which of the two it is, because a lane with `─` on both sides has the
 * horizontal running through it. */
const VERTICAL: Record<string, { up: boolean; down: boolean }> = {
  "│": { up: true, down: true },
  "┤": { up: true, down: true },
  "┴": { up: true, down: false },
  "┘": { up: true, down: false },
  "┬": { up: false, down: true },
  "┐": { up: false, down: true },
};

/** The verticals, connections and commit of one row, in lane units.
 *
 * Lane units rather than pixels: how wide a lane is and how tall a row is are
 * the panel's business, so the same geometry survives a layout change and the
 * tests can assert it without knowing any sizes. `laneDrawing` turns it into
 * pixels. */
export interface GraphGeometry {
  /** Vertical threads. `fromTop`/`toBottom` say whether the thread reaches the
   * row's own edges, which is what makes it continuous with the rows on either
   * side. */
  threads: { lane: number; fromTop: boolean; toBottom: boolean; color: string }[];
  /** A horizontal connection, merged across its lanes: `from < to`, and the
   * two are lane indices rather than the cells between them. */
  runs: { from: number; to: number; color: string }[];
  /** The commit itself, where its dot is drawn. */
  marker: { lane: number; color: string; merged: boolean } | null;
}

/**
 * A whole page of gutters as geometry, one entry per commit row.
 *
 * Decoded in one pass because a row cannot be read alone: whether a commit is
 * the *tip* of a lane (nothing arrives from above or leaves below, so no thread
 * may be drawn past its dot) is only knowable from the rows around it. A tip
 * drawn with a thread above it grows a stub that connects to nothing — the
 * stray marks that make an ASCII rendering look broken.
 *
 * Two colour rules make the picture readable rather than merely coloured, and
 * both are the TUI's (`paint_graph` in `tui/src/workbench/screen.rs`):
 *
 * * a thread and its commit wear their lane's colour, so a line of work is one
 *   colour from its tip to the commit where it joins another;
 * * a connection wears the *commit's* colour, not the colour of the lane it
 *   passes through — one connection, one colour. Repainted per segment it would
 *   read as a different branch at every lane it crosses.
 */
export function graphGeometry(rows: { graph: string }[], palette: GraphPalette): GraphGeometry[] {
  const occupied = rows.map(row => {
    const lanes: boolean[] = [];
    for (let lane = 0; lane * 2 < row.graph.length; lane++) {
      lanes[lane] = (row.graph[lane * 2] ?? " ") !== " ";
    }
    return lanes;
  });

  return rows.map((row, index) => {
    const gutter = row.graph;
    const mine = commitColumn(gutter);
    const mineLane = mine === null ? null : Math.floor(mine / 2);
    const threads: GraphGeometry["threads"] = [];

    for (let lane = 0; lane * 2 < gutter.length; lane++) {
      const glyph = gutter[lane * 2]!;
      if (glyph === " ") continue;
      const color = laneColor(palette, lane);
      if (COMMIT.has(glyph)) {
        // A commit's thread reaches up only when something was already waiting
        // on its column, and down only when another row still carries that
        // lane: otherwise this commit is the lane's tip and a thread past its
        // dot would hang off the end of the picture.
        threads.push({
          lane,
          fromTop: occupied[index - 1]?.[lane] === true,
          toBottom: occupied[index + 1]?.[lane] === true,
          color,
        });
        continue;
      }
      const vertical = VERTICAL[glyph];
      if (!vertical) continue;
      threads.push({ lane, fromTop: vertical.up, toBottom: vertical.down, color });
    }

    // Contiguous `─` cells are one connection. Merging them is what removes the
    // per-cell gaps an ASCII gutter has: the horizontal is drawn once, from the
    // lane it leaves to the lane it arrives at.
    const runs: GraphGeometry["runs"] = [];
    const color = laneColor(palette, mineLane ?? 0);
    let previousCell = -2;
    for (let cell = 1; cell < gutter.length; cell += 2) {
      if (gutter[cell] !== "─") continue;
      const lane = (cell - 1) / 2;
      const last = runs[runs.length - 1];
      // Only *adjacent* cells extend a run: two horizontals with a gap between
      // them are two connections, and joining them would draw a line through a
      // lane the host deliberately left empty.
      if (last && previousCell === cell - 2) last.to = lane + 1;
      else runs.push({ from: lane, to: lane + 1, color });
      previousCell = cell;
    }

    return {
      threads,
      runs,
      marker: mineLane === null
        ? null
        : { lane: mineLane, color: laneColor(palette, mineLane), merged: gutter[mine!] === "M" },
    };
  });
}

/** The column the commit is drawn in, in gutter characters (even), or `null`
 * for a row that somehow carries no commit glyph. */
function commitColumn(gutter: string): number | null {
  for (let index = 0; index < gutter.length; index++) {
    if (COMMIT.has(gutter[index]!)) return index;
  }
  return null;
}

function laneColor(palette: GraphPalette, lane: number): string {
  return palette[lane % palette.length] ?? palette[0]!;
}

/** How many lanes the widest row of a listing uses. Every row is padded to the
 * same width by the host, so this is also the page's lane count. */
export function laneCount(rows: { graph: string }[]): number {
  const widest = rows.reduce((most, row) => Math.max(most, row.graph.length), 0);
  return Math.ceil(widest / 2);
}

/** The measurements a lane picture is drawn with, in pixels. */
export interface LaneMetrics {
  /** Distance between two lane centres. */
  pitch: number;
  /** How far a turn reaches before it is on the centre line: also how much of
   * the vertical the corner consumes. */
  radius: number;
  /** How much room a crossing opens around the lane it passes. */
  gap: number;
}

/** One row's picture, in the pixels the panel paints. */
export interface LaneDrawing {
  /** Straight verticals, across the row's full height so they meet the rows on
   * either side exactly. */
  threads: { x: number; y1: number; y2: number; color: string }[];
  /** Horizontals as SVG path data, turns included: one path per unbroken
   * stretch of a connection. */
  runs: { path: string; color: string }[];
  dots: { x: number; color: string; merged: boolean }[];
}

/**
 * Geometry to strokes, for one row of a given height.
 *
 * Exact numbers rather than percentages, because two strokes only join if one
 * ends where the other begins: every row is deliberately the same height (see
 * the panel) so a thread can run from the row's top edge to its bottom edge and
 * meet its neighbours there.
 *
 * Three ways a horizontal can meet a lane, and why they look different:
 *
 * * the horizontal **turns into** the lane — a fork closing above (`┘`) or a
 *   lane starting below (`┐`). It gets a rounded corner, and the vertical is
 *   shortened by the radius so the turn is drawn once instead of a line ending
 *   inside another line's stroke;
 * * the lane **runs through** — a crossing. The vertical runs the whole row and
 *   the horizontal is broken with a small gap on either side, because a crossing
 *   mark would claim two lines touch when one merely passes the other;
 * * the lane **ends** there, or is the commit itself. The horizontal meets it
 *   square on the centre line. This is the merge's own lane (`┤`): the line
 *   arriving stops, the vertical carries on.
 */
export function laneDrawing(
  geometry: GraphGeometry,
  metrics: LaneMetrics,
  height: number,
): LaneDrawing {
  const center = height / 2;
  const { pitch, radius, gap } = metrics;
  const x = (lane: number) => pitch / 2 + lane * pitch;
  const threadOf = (lane: number) => geometry.threads.find(thread => thread.lane === lane);
  // The lane a continuous vertical occupies: a run may cut through it, never
  // join it. `┤` is *not* this — it is a vertical that carries on while the
  // horizontal arriving at it stops, which is a junction, not a crossing.
  const runsThrough = (lane: number) => {
    const thread = threadOf(lane);
    return thread?.fromTop === true && thread.toBottom;
  };

  const markerLane = geometry.marker?.lane ?? -1;
  // A turn, and which way: the lane the horizontal bends into. Only at the two
  // ends of a run, and never at the commit, whose dot sits on the centre line
  // where a corner would have to be.
  const turns = new Map<number, "up" | "down">();
  for (const run of geometry.runs) {
    for (const lane of [run.from, run.to]) {
      if (lane === markerLane || runsThrough(lane)) continue;
      const thread = threadOf(lane);
      if (!thread) continue;
      if (thread.fromTop && !thread.toBottom) turns.set(lane, "up");
      else if (!thread.fromTop && thread.toBottom) turns.set(lane, "down");
    }
  }

  const threads = geometry.threads.map(thread => {
    const turn = turns.get(thread.lane);
    return {
      x: x(thread.lane),
      y1: thread.fromTop ? 0 : turn === "down" ? center + radius : center,
      y2: thread.toBottom ? height : turn === "up" ? center - radius : center,
      color: thread.color,
    };
  }).filter(thread => thread.y2 > thread.y1);
  // A thread that starts and ends on the centre line has no length. It is the
  // commit at the tip of its lane, with nothing above or below it to join: the
  // dot is the whole picture, and a zero-length stroke with a round cap would
  // paint a second dot on top of it.

  const runs: LaneDrawing["runs"] = [];
  for (const run of geometry.runs) {
    // A lane in the middle of a run is one the horizontal passes rather than
    // reaches, so it is cut there — but only where the vertical is continuous;
    // where it merely starts or ends on this row the horizontal carries on,
    // which is exactly what `┬`/`┴` inside a run mean.
    const stops: { lane: number; edge: boolean }[] = [{ lane: run.from, edge: true }];
    for (let lane = run.from + 1; lane < run.to; lane++) {
      if (runsThrough(lane)) stops.push({ lane, edge: false });
    }
    stops.push({ lane: run.to, edge: true });

    for (let index = 0; index < stops.length - 1; index++) {
      const left = stops[index]!;
      const right = stops[index + 1]!;
      const end = (stop: { lane: number; edge: boolean }): RunEnd =>
        !stop.edge ? "cross" : turns.get(stop.lane) ?? "join";
      const path = runPath(left.lane, right.lane, end(left), end(right), { x, center, radius, gap });
      if (path) runs.push({ path, color: run.color });
    }
  }

  return {
    threads,
    runs,
    dots: geometry.marker
      ? [{
        x: x(geometry.marker.lane),
        color: geometry.marker.color,
        merged: geometry.marker.merged,
      }]
      : [],
  };
}

/** How a horizontal stretch meets the lane at one of its ends. */
type RunEnd =
  /** A lane it turns into, going up to meet a vertical above this row. */
  | "up"
  /** …or down to meet one below. */
  | "down"
  /** A lane it passes: the stroke stops short of it. */
  | "cross"
  /** A lane it joins square on the centre line: the commit, or a merge. */
  | "join";

/**
 * One horizontal stretch, as SVG path data.
 *
 * Quadratic curves rather than arcs: a quarter turn expressed as an arc needs a
 * sweep flag whose direction differs between the left and right end (SVG's y
 * axis points down), which is easy to write backwards and hard to notice when it
 * is. `Q` with the corner as its control point is the same shape and cannot be
 * inverted.
 *
 * A `cross` end is pulled back by `gap` so the stroke stops short of a lane it
 * passes rather than touching it; a `turn` end stops a radius short and curves
 * up to meet the vertical, which the caller has already shortened to match.
 */
function runPath(
  from: number,
  to: number,
  left: RunEnd,
  right: RunEnd,
  geometry: { x: (lane: number) => number; center: number; radius: number; gap: number },
): string {
  const { x, center, radius, gap } = geometry;
  // One decimal is below a pixel at any density and keeps the path readable in
  // a snapshot.
  const n = (value: number) => Math.round(value * 10) / 10;
  const parts: string[] = [];
  const turns = (end: RunEnd) => end === "up" || end === "down";

  let start = x(from) + (left === "cross" ? gap : 0);
  if (turns(left)) {
    // The stroke starts above (or below) the centre line, on the vertical's own
    // axis, and curves onto it: one turn rather than two lines meeting in a
    // spike.
    const begin = center + (left === "up" ? -radius : radius);
    parts.push(
      `M ${n(x(from))} ${n(begin)}`,
      `Q ${n(x(from))} ${n(center)} ${n(x(from) + radius)} ${n(center)}`,
    );
    start = x(from) + radius;
  } else {
    parts.push(`M ${n(start)} ${n(center)}`);
  }

  const end = x(to) - (right === "cross" ? gap : turns(right) ? radius : 0);
  // A stretch with no length is not a stroke: `M x y L x y` paints nothing but
  // still costs a native draw call per row, and an empty `d` is rejected by the
  // renderer. It is reachable when a run is one lane wide and both ends turn.
  if (!turns(right) && end <= start) return "";
  parts.push(`L ${n(end)} ${n(center)}`);
  if (turns(right)) {
    const finish = center + (right === "up" ? -radius : radius);
    parts.push(`Q ${n(x(to))} ${n(center)} ${n(x(to))} ${n(finish)}`);
  }
  return parts.join(" ");
}

/** One painted line of a commit's patch. */
export interface PatchLine {
  text: string;
  /** A role, not a colour: the caller maps it through the theme. */
  role: "added" | "removed" | "hunk" | "header" | "meta" | "plain";
}

/** A `git show` header field: `commit <sha>`, `Author:`, `Date:`, and the
 * variants `--pretty` can add. These name the commit rather than being it. */
const PREAMBLE_FIELD = /^(commit |Merge: |Author:|AuthorDate:|Commit:|CommitDate:|Date: )/;

/** A stat-summary row: ` file | 4 ++--`, or the `3 files changed, …` total. */
const STAT_LINE = /^(\s+\S.*\|\s+\d+|\s*\d+ files? changed)/;

/**
 * A patch split into coloured lines.
 *
 * `git show --stat --patch` prints four blocks in order: the commit preamble
 * (`commit`/`Author`/`Date` and the message), the stat summary, then the diff —
 * the `diff --git` line, the `index`/`---`/`+++` trio, hunks (`@@`), additions,
 * removals and context. Everything before the first `diff --git` is therefore
 * *about* the commit rather than part of it, and is split by shape: the header
 * fields and the stat are metadata, and the message body reads as prose.
 *
 * The roles are kept apart from the colours so a theme decides what "added"
 * looks like, which is the same split `codeHighlight` makes for source tokens.
 *
 * Deliberately *not* `git`'s `--color`: that embeds ANSI escapes in the text the
 * reader selects and copies, and it says nothing about which lines matter once
 * the terminal codes are gone.
 */
export function patchLines(patch: string): PatchLine[] {
  const lines = patch.split("\n");
  // A patch ends with a newline, so the final element is empty and painting it
  // would open a blank line under every commit.
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  let inDiff = false;
  return lines.map(text => {
    if (!inDiff) {
      // The diff begins at the first `diff --git`; until then every line is
      // preamble, and this one is the boundary.
      if (text.startsWith("diff --git ")) inDiff = true;
      else if (PREAMBLE_FIELD.test(text) || STAT_LINE.test(text)) {
        return { text, role: "meta" };
      } else return { text, role: "plain" };
    }
    if (text.startsWith("diff --git ") || text.startsWith("@@")) {
      return { text, role: text.startsWith("@@") ? "hunk" : "header" };
    }
    if (
      text.startsWith("+++")
      || text.startsWith("---")
      || text.startsWith("index ")
      || text.startsWith("new file mode")
      || text.startsWith("deleted file mode")
      || text.startsWith("similarity index")
      || text.startsWith("rename ")
      || text.startsWith("old mode")
      || text.startsWith("new mode")
      || text.startsWith("Binary files")
      || text.startsWith("\\ No newline")
    ) {
      return { text, role: "header" };
    }
    if (text.startsWith("+")) return { text, role: "added" };
    if (text.startsWith("-")) return { text, role: "removed" };
    return { text, role: "plain" };
  });
}
