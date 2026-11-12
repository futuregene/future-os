import { commitLane, graphSpans, gutterWidth, patchLines } from "../gitGraph";

/** A palette whose entries are their own index, so a span's colour says which
 * lane it came from without the test having to know the theme's hex values. */
const palette = ["L0", "L1", "L2", "L3"];

/** A span list flattened back to `[text, colour]`, colours as lane indices. */
const spans = (gutter: string) =>
  graphSpans(gutter, palette).map(span => [span.text, span.color === "" ? null : palette.indexOf(span.color)]);

/**
 * The gutter is coloured by lane, and the two rules that make it readable are the
 * TUI's.
 *
 * A lane keeps one colour for as long as it keeps one column, so a line of work
 * can be followed down the page; a horizontal wears the *commit's* colour rather
 * than the colour of the column it crosses, so one connection reads as one
 * connection instead of a new branch at every elbow. Both are asserted on the
 * gutters the host really prints for those histories
 * (`host/src/remote_host/git_graph/tests.rs` draws them from a real repository).
 */
describe("graphSpans", () => {
  test("a commit glyph wears its own lane's colour", () => {
    // Two lanes, the commit in the left one.
    expect(spans("o │ ")).toEqual([["o", 0], [" ", null], ["│", 1], [" ", null]]);
  });

  test("a commit on the right lane wears that lane, not the leftmost", () => {
    expect(spans("│ o ")).toEqual([["│", 0], [" ", null], ["o", 1], [" ", null]]);
  });

  test("the horizontal of a branch wears the commit's colour, not the column's", () => {
    // The merge row: a commit on lane 0 opening lane 1. The `─` between them is
    // the commit's connection, so it takes lane 0's colour; the `┐` that turns
    // into lane 1 is that lane's own corner, as the TUI paints it.
    expect(spans("M─┐ ")).toEqual([["M─", 0], ["┐", 1], [" ", null]]);
  });

  test("a merge commit is a commit glyph like any other", () => {
    expect(spans("M").find(([, lane]) => lane === 0)).toBeDefined();
    // …and it is what `commitLane` reads, so the row's stripe agrees with the
    // colour its marker wears.
    expect(commitLane("M─┐ ")).toBe(0);
  });

  test("three lanes keep three colours down the page", () => {
    // From the three-lane fixture: each row's trunk and branches stay distinct,
    // so a reader can follow one line of work through a crossing.
    expect(spans("│ M─┐ ")).toEqual([["│", 0], [" ", null], ["M─", 1], ["┐", 2], [" ", null]]);
    // The second parent's own commit sits on lane 2 and wears that lane.
    expect(spans("│ │ o ")[4]).toEqual(["o", 2]);
  });

  test("a horizontal closing onto the trunk is one run of the commit's colour", () => {
    // `o───┘`: the trunk, the whole crossing, and the corner that ends it — the
    // `─`s are the commit's own connection and stay one colour, so a merge does
    // not read as adopting the branch it closes.
    expect(spans("o───┘ ")).toEqual([["o───", 0], ["┘", 2], [" ", null]]);
  });

  test("a row with no commit glyph still draws, uncoloured where it has no lane", () => {
    // `commitColumn` returns null, so there is no "commit's colour" to use; the
    // drawing must still come out aligned rather than collapsing.
    expect(spans("  ")).toEqual([["  ", null]]);
    expect(commitLane("  ")).toBe(0);
  });

  test("pad runs of the same colour merge, so one lane is one Text", () => {
    // A lane crossing four rows of one column is a single span per row — the
    // reason a 200-row listing does not become thousands of native views.
    const wide = graphSpans("│ │ │ ", palette);
    expect(wide).toHaveLength(6);
    expect(wide.every(span => span.text.length === 1)).toBe(true);
  });
});

describe("gutterWidth", () => {
  test("is the longest row's gutter, so every row shares one column width", () => {
    expect(gutterWidth([{ graph: "o " }, { graph: "M─┐ " }, { graph: "│ │ o " }])).toBe(6);
  });

  test("an empty listing is zero wide rather than NaN", () => {
    expect(gutterWidth([])).toBe(0);
  });
});

describe("commitLane", () => {
  test("is the column the commit is drawn in, divided by two", () => {
    expect(commitLane("o ")).toBe(0);
    expect(commitLane("│ o ")).toBe(1);
    expect(commitLane("│ │ o ")).toBe(2);
  });
});

/**
 * A patch is coloured by line role, not by git's own ANSI.
 *
 * Git's `--color` would ride along in the text the reader selects and copies, and
 * it says nothing about which lines matter once the escapes are gone. The roles
 * are kept apart from the colours so the theme decides what "added" looks like.
 */
describe("patchLines", () => {
  const patch = [
    " analysis/compare.R | 4 ++--",
    " 1 file changed, 2 insertions(+), 2 deletions(-)",
    "",
    "diff --git a/analysis/compare.R b/analysis/compare.R",
    "index 8d2a551..f30bc92 100644",
    "--- a/analysis/compare.R",
    "+++ b/analysis/compare.R",
    "@@ -18,8 +18,8 @@",
    " context line",
    "-  adapt_delta = 0.80,",
    "+  adapt_delta = 0.95,",
    "",
  ].join("\n");

  test("the stat block before the diff is metadata, and the diff is roles", () => {
    expect(patchLines(patch)).toEqual([
      { text: " analysis/compare.R | 4 ++--", role: "meta" },
      { text: " 1 file changed, 2 insertions(+), 2 deletions(-)", role: "meta" },
      { text: "", role: "plain" },
      { text: "diff --git a/analysis/compare.R b/analysis/compare.R", role: "header" },
      { text: "index 8d2a551..f30bc92 100644", role: "header" },
      { text: "--- a/analysis/compare.R", role: "header" },
      { text: "+++ b/analysis/compare.R", role: "header" },
      { text: "@@ -18,8 +18,8 @@", role: "hunk" },
      { text: " context line", role: "plain" },
      { text: "-  adapt_delta = 0.80,", role: "removed" },
      { text: "+  adapt_delta = 0.95,", role: "added" },
    ]);
  });

  test("a patch's trailing newline does not open a blank line under every commit", () => {
    // The failing-real-world shape: a diff whose only line is an addition. The
    // trailing newline is the file's, not a line of the patch.
    const lines = patchLines("diff --git a/x b/x\n+only line\n");
    expect(lines).toEqual([
      { text: "diff --git a/x b/x", role: "header" },
      { text: "+only line", role: "added" },
    ]);
  });

  test("a diff header is a header even when the commit message follows it", () => {
    // `git show` prints the preamble first: the header fields and the stat are
    // metadata, the message body is prose, and only then does the diff begin.
    const lines = patchLines(
      "commit abc123\nAuthor: Lixin <l@example.org>\n\n    Subject\n\n x.R | 2 +-\ndiff --git a/x b/x\n",
    );
    expect(lines.map(line => line.role)).toEqual([
      "meta", "meta", "plain", "plain", "plain", "meta", "header",
    ]);
  });

  test("an empty patch paints nothing at all", () => {
    expect(patchLines("")).toEqual([]);
  });
});
