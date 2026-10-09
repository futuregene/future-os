import type { Nodes, Root } from "mdast";
import type { Plugin } from "unified";

/**
 * Recover a link whose angle-bracket destination is missing its closing `>`.
 *
 * A model writing a link to a file emits `[bench/REPORT.md](<./bench/REPORT.md>)`
 * — but quite often drops the `>` and writes `](<./bench/REPORT.md)`. CommonMark
 * only accepts `<…>` as a destination when both brackets are present, so no link
 * forms at all: `[bench/REPORT.md]` stays bracketed text (and the bracketed-path
 * mention pass turns it into a file chip), while the remaining `(<./bench/REPORT.md)`
 * renders as literal text. The reader sees the path twice —
 * `bench/REPORT.md(<./bench/REPORT.md)` — which is what this repairs.
 *
 * The mdast transform rebuilds the text node holding the broken link into
 * `text` + `link` + `text`, so the ordinary link conversion (and its
 * file-reference chip) takes over. Like `remarkAutolinkBoundary` it never
 * rewrites the source: code spans, escapes, real link destinations and the
 * streaming projector's source offsets are untouched. A broken link whose label
 * carries inline markup (`[**x**](<…)`) spans several nodes and stays literal,
 * the same limitation the autolink pass documents.
 */
export const remarkUnclosedLink: Plugin<[], Root> = function () {
  const data = this.data();
  (data.fromMarkdownExtensions ??= []).push({ transforms: [repairUnclosedLinks] });
};

/**
 * `[label](<destination)` — a destination that opens with `<` and closes with
 * `)` instead of `>`. The destination excludes `>`/`)`/whitespace so a
 * well-formed `](<destination>)` never matches (its `>` blocks the `)`) and the
 * match cannot run past the closing parenthesis.
 */
const unclosedLink = /\[([^[\]\n]+)\]\(<([^\s<>()\n]+)\)/g;

/** Chunks of `value` (text and links), or `null` when it holds no broken link
 * so the caller keeps the node's identity. */
function repairInText(value: string): Nodes[] | null {
  unclosedLink.lastIndex = 0;
  const parts: Nodes[] = [];
  let end = 0;
  for (let match = unclosedLink.exec(value); match; match = unclosedLink.exec(value)) {
    const [whole, label, url] = match as unknown as [string, string, string];
    if (match.index > end) parts.push({ type: "text", value: value.slice(end, match.index) });
    parts.push({ type: "link", url, children: [{ type: "text", value: label }] });
    end = match.index + whole.length;
  }
  if (parts.length === 0) return null;
  if (end < value.length) parts.push({ type: "text", value: value.slice(end) });
  return parts;
}

/** The mdast transform: walk every text node, rebuilding the ones that hold a
 * broken link. Untouched nodes keep their identity. */
function repairUnclosedLinks(parent: Nodes): void {
  if (!("children" in parent)) return;
  const children = parent.children as Nodes[];
  let repaired: Nodes[] | null = null;
  for (const [index, child] of children.entries()) {
    const parts = child.type === "text" ? repairInText(child.value) : null;
    if (parts) {
      repaired ??= children.slice(0, index);
      repaired.push(...parts);
    } else {
      repaired?.push(child);
      repairUnclosedLinks(child);
    }
  }
  if (repaired) parent.children = repaired as typeof parent.children;
}
