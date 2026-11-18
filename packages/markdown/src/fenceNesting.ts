/**
 * A fenced block that *quotes* a document containing a fenced block.
 *
 * CommonMark gives a fence no way to nest: a ``` run closes whichever fence it
 * is inside, so the shape a model reaches for when it quotes a whole Markdown
 * file —
 *
 *     ```markdown
 *     # Title
 *
 *     ```futureos-choice
 *     title: T
 *     ```
 *
 *     - a list that follows the card
 *     ```
 *
 * — loses its body at the inner fence. The outer block ends after
 * `title: T`, and everything the author meant to be *inside* it (the list, the
 * following paragraphs) is parsed as the reply's own Markdown. Measured on a
 * real reply: the list under the quoted file rendered as untitled bullets in
 * the middle of the message, the `## heading` after it disappeared into a
 * second empty code block, and a shell transcript below was read as prose with
 * its `$(…)` turned into an inline formula.
 *
 * The author's intent is unambiguous — a document fence contains other fences —
 * so this reads those fences as nested instead. Only fences whose info string
 * names a document language (`markdown`, `md`) get the treatment: for every
 * other language a ` ``` ` inside the body is ordinary text, which CommonMark
 * already handles.
 *
 * The repair is applied to the *source* and is length-preserving, which is what
 * makes it safe here: `streamingMarkdown` and the desktop projector slice the
 * reply by the mdast `position` offsets this source produces, so inserting a
 * character would shift every block boundary after it. Instead of lengthening
 * the outer fence past its body (the usual way to write this by hand), the
 * outer delimiters are swapped to tildes of the same width — a backtick run
 * inside a tilde fence is content, and a tilde run inside a backtick fence is
 * content — so the same characters stay at the same offsets and only the
 * grouping changes. The delimiters themselves are never part of the rendered
 * code, so the block's text is exactly what the author wrote.
 */

/** ` ``` ` or `~~~`, up to three spaces in, plus everything after the run. */
const FENCE = /^( {0,3})(`{3,}|~{3,})(.*)$/;

/** Info strings that name a document rather than a programming language. */
const DOCUMENT_LANGUAGES = new Set(["markdown", "md"]);

export interface FenceLine {
  /** Backtick or tilde. */
  char: "`" | "~";
  /** Length of the opening run. */
  length: number;
  /** First non-space character after the run, or "" for a bare run. */
  info: string;
}

export function readFenceLine(line: string): FenceLine | null {
  const match = FENCE.exec(line);
  if (!match) return null;
  const [, , run = "", rest = ""] = match;
  const info = rest.trim();
  // A backtick fence's info string may not contain a backtick (CommonMark), so
  // `` `a `b` `` is inline code, not a fence. Tilde fences have no such rule.
  if (run[0] === "`" && info.includes("`")) return null;
  return { char: run[0] as "`" | "~", length: run.length, info };
}

export function closesFence(line: string, open: FenceLine): boolean {
  const match = FENCE.exec(line);
  if (!match) return false;
  const [, , run = "", rest = ""] = match;
  return run[0] === open.char && run.length >= open.length && rest.trim() === "";
}

/**
 * Index of the line that ends a document fence if its body is read as nested
 * Markdown, or -1 when there is no such line.
 *
 * Depth starts at the outer fence. A line opening a fence with an info string
 * nests one level deeper; a bare run closes the innermost level. The outer fence
 * ends when the depth returns to zero — which is what the author wrote, and what
 * a file copied between two ``` runs looks like.
 */
function nestedClose(lines: string[], openIndex: number, open: FenceLine): number {
  let depth = 1;
  for (let index = openIndex + 1; index < lines.length; index += 1) {
    const fence = readFenceLine(lines[index]!);
    if (!fence) continue;
    // Any tilde run of fence width in the body would close the rewritten outer
    // fence early, so this shape cannot be expressed by the swap below.
    if (fence.char === "~" && fence.length >= 3) return -1;
    if (fence.info !== "") {
      depth += 1;
    } else if (fence.char === open.char && fence.length >= open.length) {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

/** Index of the line CommonMark would end this fence on, or -1. */
function plainClose(lines: string[], openIndex: number, open: FenceLine): number {
  for (let index = openIndex + 1; index < lines.length; index += 1) {
    if (closesFence(lines[index]!, open)) return index;
  }
  return -1;
}

/** Swap the leading run of one fence line for the same number of tildes. */
function toTildes(line: string): string {
  const match = FENCE.exec(line);
  /* v8 ignore next -- only called for lines readFenceLine already matched */
  if (!match) return line;
  const [, indent = "", run = "", rest = ""] = match;
  return indent + "~".repeat(run.length) + rest;
}

/**
 * Read a document fence as containing other fences. Returns `source` unchanged
 * (same identity) when there is nothing to repair.
 */
export function repairNestedDocumentFence(source: string): string {
  if (!source.includes("```")) return source;
  const lines = source.split("\n");
  let changed = false;
  let index = 0;
  while (index < lines.length) {
    const open = readFenceLine(lines[index]!);
    if (!open) {
      index += 1;
      continue;
    }
    const plain = plainClose(lines, index, open);
    if (plain < 0) {
      // Unterminated: nothing follows that could be mis-grouped.
      break;
    }
    if (open.char === "`" && DOCUMENT_LANGUAGES.has(open.info.toLowerCase())) {
      const nested = nestedClose(lines, index, open);
      // `nested !== plain` is the whole signal: CommonMark stopped at an inner
      // fence's closing run, so the body runs further than it read.
      if (nested > plain) {
        lines[index] = toTildes(lines[index]!);
        lines[nested] = toTildes(lines[nested]!);
        changed = true;
        // Keep scanning after the fence that now owns this body.
        index = nested + 1;
        continue;
      }
    }
    index = plain + 1;
  }
  return changed ? lines.join("\n") : source;
}
