import type { GraphPalette } from "../../theme/tokens";

/** One painted run of the graph gutter: its text and the colour it wears.
 * Adjacent runs of the same colour are merged, so a lane's vertical is one
 * `Text` rather than one per cell. */
export interface GraphSpan {
  text: string;
  color: string;
}

/** The glyphs a commit row's gutter is built from, as the host writes them
 * (`host/src/remote_host/git_graph.rs`, the TUI's vocabulary). */
const COMMIT = new Set(["o", "M"]);
const VERTICAL = new Set(["│"]);
const ELBOW = new Set(["┐", "┘", "┤", "┬", "┴"]);
const HORIZONTAL = new Set(["─"]);

/**
 * The gutter split into coloured runs, one lane per colour.
 *
 * A lane sits at character `2n` and the character to its right carries the
 * connection toward the next lane, so a glyph's lane is its column divided by
 * two: a branch keeps one colour for as long as it keeps one column. Two rules
 * make the picture readable rather than merely coloured, and both are the TUI's
 * (`paint_graph` in `tui/src/workbench/screen.rs`):
 *
 * * a commit glyph wears its lane's colour at full strength, so a line of work
 *   is one colour from its tip to the commit where it joins another;
 * * a horizontal wears the *commit's* colour, not the colour of the column it
 *   passes through — one connection, one colour. A horizontal repainted per
 *   segment would read as a different branch at every elbow it crosses.
 *
 * Counting characters is counting cells here because the gutter is this
 * module's own vocabulary: the subject travels in its own field and never
 * reaches this string, so no free text can shift a column.
 */
export function graphSpans(gutter: string, palette: GraphPalette): GraphSpan[] {
  const mine = commitColumn(gutter);
  const spans: GraphSpan[] = [];
  for (let index = 0; index < gutter.length; index++) {
    const glyph = gutter[index]!;
    const lane = Math.floor(index / 2);
    const color = colorOf(glyph, lane, mine, palette);
    if (color === null) {
      // A space, or a glyph this vocabulary does not know yet: leave it
      // uncoloured rather than dropping it, so the drawing stays aligned.
      if (!spans.length || spans[spans.length - 1]!.color !== "") {
        spans.push({ text: glyph, color: "" });
      } else {
        spans[spans.length - 1]!.text += glyph;
      }
      continue;
    }
    const last = spans[spans.length - 1];
    if (last && last.color === color) last.text += glyph;
    else spans.push({ text: glyph, color });
  }
  return spans;
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

function colorOf(
  glyph: string,
  lane: number,
  mine: number | null,
  palette: GraphPalette,
): string | null {
  if (COMMIT.has(glyph)) return laneColor(palette, lane);
  if (VERTICAL.has(glyph)) return laneColor(palette, lane);
  if (ELBOW.has(glyph)) return laneColor(palette, lane);
  if (HORIZONTAL.has(glyph)) return laneColor(palette, mine === null ? lane : Math.floor(mine / 2));
  return null;
}

/** How wide the gutter is, in characters: the longest row of a listing. */
export function gutterWidth(rows: { graph: string }[]): number {
  return rows.reduce((widest, row) => Math.max(widest, row.graph.length), 0);
}

/** The lane a row's commit belongs to — the colour the row's marker wears, and
 * what a selected row's band is tinted with. */
export function commitLane(gutter: string): number {
  const column = commitColumn(gutter);
  return column === null ? 0 : Math.floor(column / 2);
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
