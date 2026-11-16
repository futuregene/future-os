import { describe, expect, it } from "vitest";
import { graphGeometry, laneCount, laneDrawing, patchLines } from "./index";
import type { LaneMetrics } from "./index";

/** A palette whose entries are their own index, so a stroke's colour says which
 * lane it came from without the test having to know the theme's hex values. */
const palette = ["L0", "L1", "L2", "L3"];

const metrics: LaneMetrics = { pitch: 14, radius: 6, gap: 3 };
const HEIGHT = 34;
const CENTER = HEIGHT / 2;

const rows = (...graph: string[]) => graph.map(line => ({ graph: line }));
const geometry = (...graph: string[]) => graphGeometry(rows(...graph), palette);
const draw = (...graph: string[]) => {
  const decoded = geometry(...graph);
  return decoded.map(row => laneDrawing(row, metrics, HEIGHT));
};
const thread = (row: { threads: { lane: number }[] }, lane: number) =>
  row.threads.find(entry => entry.lane === lane)!;

/**
 * The gutters below are what the host really prints, not invented ones: they are
 * the output of `remote_host::git_graph::list` for a repository whose history is
 * two branches merged back into `main` (`host/src/remote_host/git_graph/tests.rs`
 * builds the same fixture through git itself). A picture asserted against made-up
 * gutters would agree with the decoder and disagree with the desktop.
 */
const MERGED = ["o     ", "│ M─┐ ", "│ │ o ", "│ o │ ", "o─┴─┘ "] as const;

/**
 * The gutter is decoded into strokes, and the two rules that make the picture
 * readable are the TUI's.
 *
 * A lane keeps one colour for as long as it keeps one column, so a line of work
 * can be followed down the page; a horizontal wears the *commit's* colour rather
 * than the colour of the column it crosses, so one connection reads as one
 * connection instead of a new branch at every elbow.
 */
describe("graphGeometry", () => {
  it("a commit's thread wears its own lane's colour", () => {
    const [commit] = geometry("o │ ");
    expect(commit!.marker).toEqual({ lane: 0, color: "L0", merged: false });
    expect(commit!.threads.map(entry => [entry.lane, entry.color])).toEqual([
      [0, "L0"],
      [1, "L1"],
    ]);
  });

  it("a commit on the right lane wears that lane, not the leftmost", () => {
    expect(geometry("│ o ")[0]!.marker).toEqual({ lane: 1, color: "L1", merged: false });
  });

  it("a merge is marked as one, so the row can draw a ring instead of a disc", () => {
    expect(geometry(...MERGED)[1]!.marker).toEqual({ lane: 1, color: "L1", merged: true });
  });

  it("a connection wears the commit's colour, not the colour of the lane it enters", () => {
    // The merge row: `M` on lane 1 opening lane 2. The horizontal is the merge
    // commit's own connection, so all of it is lane 1's colour — painted per
    // lane it would read as a different branch either side of the elbow.
    expect(geometry(...MERGED)[1]!.runs).toEqual([{ from: 1, to: 2, color: "L1" }]);
  });

  it("a connection across two lanes is one run, not two collinear ones", () => {
    // `o─┴─┘`: lane 0's commit connects to lane 2. The `┴` between them is a
    // lane that ends on this row, so the horizontal carries on across it — and
    // painted as two runs there would be a seam at the junction.
    expect(geometry(...MERGED)[4]!.runs).toEqual([{ from: 0, to: 2, color: "L0" }]);
  });

  it("a thread reaches only as far as there is a lane to reach", () => {
    // The newest commit: nothing above it, and lane 0 carries on below it.
    const tip = geometry(...MERGED)[0]!;
    expect(thread(tip, 0)).toMatchObject({ fromTop: false, toBottom: true });
    // `side work` is lane 2's only commit, so its thread runs up to the row's
    // top edge and stops at its own dot — no stub hanging off the end.
    const side = geometry(...MERGED)[2]!;
    expect(thread(side, 2)).toMatchObject({ fromTop: true, toBottom: true });
  });
});

/**
 * The geometry is in lane units and the drawing is in pixels, so the numbers
 * below are the ones the panel paints. They are worth pinning: two strokes only
 * join if one ends exactly where the other begins, and a rounded turn consumes
 * as much of the vertical as it bends through, or the corner gets drawn twice
 * and reads as a kink.
 *
 * Lane centres are `pitch/2 + lane*pitch`, so 7, 21, 35 for three lanes.
 */
describe("laneDrawing", () => {
  it("verticals run the row's full height so consecutive rows join up", () => {
    expect(draw(...MERGED)[2]!.threads).toEqual([
      { x: 7, y1: 0, y2: HEIGHT, color: "L0" },
      { x: 21, y1: 0, y2: HEIGHT, color: "L1" },
      { x: 35, y1: 0, y2: HEIGHT, color: "L2" },
    ]);
  });

  it("a lane starting below gets a rounded corner, not a right angle", () => {
    // `┐`: the lane exists only below this row, so the horizontal bends down
    // into it. The corner curves onto the lane's axis, and the vertical starts
    // where the curve ends — starting it at the centre line would draw the
    // corner twice.
    const merge = draw(...MERGED)[1]!;
    expect(merge.runs).toEqual([
      { path: "M 21 17 L 29 17 Q 35 17 35 23", color: "L1" },
    ]);
    expect(merge.threads.find(entry => entry.x === 35)).toEqual(
      { x: 35, y1: CENTER + metrics.radius, y2: HEIGHT, color: "L2" },
    );
  });

  it("a fork closing above is the mirror: it bends up and stops short", () => {
    // `┘`: the lane exists only above this row, so the corner rises out of the
    // centre line and the vertical ends where the curve begins.
    const base = draw(...MERGED)[4]!;
    expect(base.runs).toEqual([
      { path: "M 7 17 L 29 17 Q 35 17 35 11", color: "L0" },
    ]);
    expect(base.threads.find(entry => entry.x === 35)).toEqual(
      { x: 35, y1: 0, y2: CENTER - metrics.radius, color: "L2" },
    );
  });

  it("a crossing keeps its thread whole and breaks the stroke around it", () => {
    // A lane in the middle of a run is one the horizontal *passes*. Its thread
    // runs the full row and the stroke is cut either side of it, so the picture
    // reads as one line crossing another rather than as a `+` junction. The two
    // pieces are separate strokes, not one path with a gap: a gap would have to
    // be drawn as a dash pattern, which cannot know where the lanes are.
    const crossing = laneDrawing(
      {
        threads: [{ lane: 1, fromTop: true, toBottom: true, color: "L1" }],
        runs: [{ from: 0, to: 2, color: "L0" }],
        marker: null,
      },
      metrics,
      HEIGHT,
    );
    expect(crossing.threads).toEqual([{ x: 21, y1: 0, y2: HEIGHT, color: "L1" }]);
    // Each piece stops `gap` short of the vertical it passes, and the stroke is
    // the commit's colour on both sides.
    expect(crossing.runs).toEqual([
      { path: "M 7 17 L 18 17", color: "L0" },
      { path: "M 24 17 L 35 17", color: "L0" },
    ]);
    expect(crossing.runs.every(run => !run.path.includes("Q"))).toBe(true);
  });

  it("a commit is a disc at the tip of its lane, with no thread over its dot", () => {
    const tip = draw(...MERGED)[0]!;
    expect(tip.dots).toEqual([{ x: 7, y: CENTER, color: "L0", merged: false }]);
    // Nothing above the newest commit, so its thread starts at the dot and runs
    // down to meet the next row.
    expect(tip.threads).toEqual([{ x: 7, y1: CENTER, y2: HEIGHT, color: "L0" }]);
  });

  it("a commit alone in its lane paints no zero-length thread over its dot", () => {
    // A stroke whose start and end coincide paints nothing, but with a round cap
    // it still paints — a second dot, slightly offset, on top of the real one.
    const lone = laneDrawing(
      {
        threads: [{ lane: 0, fromTop: false, toBottom: false, color: "L0" }],
        runs: [],
        marker: { lane: 0, color: "L0", merged: false },
      },
      metrics,
      HEIGHT,
    );
    expect(lone.threads).toEqual([]);
    expect(lone.dots).toEqual([{ x: 7, y: CENTER, color: "L0", merged: false }]);
  });

  it("an explicit centre moves the dot and the horizontals, never the threads", () => {
    // A row that stacks a subject and a metadata line puts its dot on the
    // first line: the panel passes that line's centre rather than the row's
    // middle. Everything that runs across the row follows it, and the verticals
    // do not — they still reach both edges, which is what keeps consecutive rows
    // joined whatever axis their dots are on.
    const decoded = geometry(...MERGED);
    const offset = laneDrawing(decoded[0]!, metrics, HEIGHT, 10);
    expect(offset.dots).toEqual([{ x: 7, y: 10, color: "L0", merged: false }]);
    expect(offset.threads).toEqual([{ x: 7, y1: 10, y2: HEIGHT, color: "L0" }]);

    const merge = laneDrawing(decoded[1]!, metrics, HEIGHT, 10);
    // The corner keeps its radius around the new axis: 4 + 6 = 10 is where the
    // curve lands on the lane's own axis.
    expect(merge.runs).toEqual([{ path: "M 21 10 L 29 10 Q 35 10 35 16", color: "L1" }]);
    expect(merge.threads.find(entry => entry.x === 35)).toEqual(
      { x: 35, y1: 16, y2: HEIGHT, color: "L2" },
    );
  });

  it("a run with no length between two turns paints no stroke", () => {
    for (const row of draw(...MERGED)) {
      for (const run of row.runs) {
        expect(run.path).not.toMatch(/^M ([\d.-]+) ([\d.]+) L \1 \2$/);
      }
    }
  });

  it("every row of a real history draws something, so no row is a gap", () => {
    for (const row of draw(...MERGED)) {
      expect(row.threads.length + row.runs.length).toBeGreaterThan(0);
    }
  });
});

describe("laneCount", () => {
  it("is the width of the widest row", () => {
    expect(laneCount(rows("o ", "│ │ o "))).toBe(3);
    expect(laneCount(rows(...MERGED))).toBe(3);
  });

  it("an empty listing has no lanes at all", () => {
    expect(laneCount([])).toBe(0);
  });
});

/**
 * The patch is coloured by line role rather than by git's ANSI, which would ride
 * along in the text the reader copies. The roles are what matters here; the
 * colours are the theme's.
 */
describe("patchLines", () => {
  it("splits a real patch into roles", () => {
    const patch = [
      "commit 4c1a7e2",
      "Author: Lixin Zhou <lixin@example.com>",
      "Date:   Mon Nov 11 18:42:00 2026 +0800",
      "",
      "    Merge the tolerance fix",
      "",
      " x.R | 2 +-",
      " 1 file changed, 1 insertion(+), 1 deletion(-)",
      "",
      "diff --git a/x.R b/x.R",
      "--- a/x.R",
      "+++ b/x.R",
      "@@ -1 +1 @@",
      "-old",
      "+new",
      "",
    ].join("\n");
    expect(patchLines(patch).map(line => line.role)).toEqual([
      "meta", "meta", "meta", "plain", "plain", "plain",
      "meta", "meta", "plain",
      "header", "header", "header", "hunk", "removed", "added",
    ]);
  });

  it("the diff's own header fields are header, not metadata", () => {
    expect(patchLines("diff --git a/x b/x\nindex 111..222 100644\n--- a/x\n+++ b/x\n")
      .map(line => line.role)).toEqual(["header", "header", "header", "header"]);
  });

  it("a trailing newline does not paint an empty line under every commit", () => {
    expect(patchLines("a\n")).toHaveLength(1);
  });

  it("prose in the commit message reads as prose, not as a header field", () => {
    expect(patchLines("commit abc\n\nAuthor notes go here\n").map(line => line.role))
      .toEqual(["meta", "plain", "plain"]);
  });
});
