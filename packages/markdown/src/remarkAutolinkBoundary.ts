import type { Nodes, Root } from "mdast";
import type { Plugin } from "unified";

/**
 * Where a bare URL (a GFM autolink literal) starts and ends.
 *
 * GFM links a bare `https://…` / `www.…` up to the next whitespace, trimming
 * only trailing ASCII punctuation — a rule written for Latin prose, where a URL
 * is always followed by a space. Chinese and Japanese prose put punctuation
 * straight after the word, so `见 https://x.com/a。下一句` swallowed the rest of
 * the sentence into the link target (tapping it opened `…/a。下一句`), and
 * `**https://x.com/a**（PRIVATE）` kept the closing `**` inside the target —
 * which both broke the target and left the emphasis unclosed, so the asterisks
 * showed up as literal text and the whole sentence was painted as a link.
 *
 * Two changes, both at the syntax-token level:
 *
 * 1. Turn off GFM's autolink *tokenizer* (`protocolAutolink`/`wwwAutolink`/
 *    `emailAutolink`). It runs before inline syntax is resolved, so everything
 *    up to the next space — the emphasis markers, code spans and punctuation
 *    that follow the URL — became part of the link and never came back. The
 *    node-scoped pass remark-gfm keeps (which runs after emphasis and link
 *    resolution) links the same URLs inside a single text node, so surrounding
 *    markup is no longer touched at all.
 * 2. Cut the text nodes that pass still sees, at both ends of the URL: after it
 *    at whitespace or full-width punctuation (`见 https://x.com/a。下一句`), and
 *    before it where GFM's pass is stricter than the tokenizer it replaces
 *    (`网页https://…` is written without a space in Chinese).
 *
 * This operates on text nodes and syntax tokens, never on rewritten source:
 * code spans, escapes, link destinations and streaming source offsets are
 * untouched. Bare URLs containing inline syntax (a literal backtick, `__`, …)
 * are the one thing the tokenizer did protect and this pass cannot see; they
 * stay literal instead of becoming a link.
 *
 * Register it *before* `remark-gfm`: its text-node pass has to run before GFM's
 * autolink pass turns a URL into a link node, or the cut lands inside the link.
 */
export const remarkAutolinkBoundary: Plugin<[], Root> = function () {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({
    disable: { null: ["protocolAutolink", "wwwAutolink", "emailAutolink"] },
  });
  (data.fromMarkdownExtensions ??= []).push({ transforms: [splitBareUrls] });
};

/**
 * Full-width/CJK punctuation, the same ranges `remarkCjkEmphasis` treats as
 * sentence punctuation (`：` `。` `、` `）` `「` `」` `“` `…`). A URL never ends
 * with one, but written Chinese puts it straight after the last character.
 */
const punctuation = /[\u2010-\u201F\u2026\u3001-\u303F\uFF01-\uFF0F\uFF1A-\uFF20\uFF3B-\uFF40\uFF5B-\uFF65]/u;

/** Start of the literals GFM links. As loose as the URL regex it feeds: a wrong
 * cut costs nothing when `findUrl` would not have linked the match anyway. */
const urlStart = /(?:https?:\/\/|www\.)/gi;

function endsBareUrl(value: string, index: number): boolean {
  const char = value[index]!;
  if (punctuation.test(char) || /\s/u.test(char)) return true;
  // A `**` run is emphasis, never URL path: it is what closes the bold that
  // opened before the URL. A single `*` stays, and so does any `_` run — those
  // do occur in URLs (`/pkg/__init__.py`).
  return char === "*" && value[index + 1] === "*";
}

/** Whether GFM's own pass links a URL that follows `char`: the start of the
 * node, whitespace, punctuation or a symbol. It is stricter than the tokenizer
 * this plugin turns off, which also linked a URL glued to a CJK letter
 * (`网页https://…` is how Chinese is written) while refusing one glued to an
 * ASCII alphanumeric (`abchttps://…` is not a URL). Cutting before the first
 * case gives the URL its own text node, where GFM accepts it again. */
function needsOwnNode(char: string): boolean {
  if (/[\s\p{P}\p{S}]/u.test(char)) return false;
  return !/[\x00-\x7F]/.test(char);
}

/** Chunks of `value`, cut at both ends of every bare URL that is glued to the
 * text around it. `null` when the node needs no cut, so callers keep their
 * node identity. */
function splitBareUrlsInText(value: string): string[] | null {
  const cuts: number[] = [];
  urlStart.lastIndex = 0;
  for (let match = urlStart.exec(value); match; match = urlStart.exec(value)) {
    const start = match.index;
    if (start > 0 && needsOwnNode(value[start - 1]!)) cuts.push(start);
    let end = start + match[0].length;
    while (end < value.length && !endsBareUrl(value, end)) end++;
    // A URL that runs to the end of the node is already bounded.
    if (end >= value.length) continue;
    cuts.push(end);
    urlStart.lastIndex = end;
  }
  if (cuts.length === 0) return null;
  const parts: string[] = [];
  let previous = 0;
  for (const cut of cuts) {
    if (cut <= previous) continue;
    parts.push(value.slice(previous, cut));
    previous = cut;
  }
  if (previous < value.length) parts.push(value.slice(previous));
  return parts;
}

/** The mdast transform: walk every text node, cutting the ones that hold a bare
 * URL glued to their neighbours. Untouched nodes keep their identity. */
function splitBareUrls(parent: Nodes): void {
  if (!("children" in parent)) return;
  const children = parent.children as Nodes[];
  let split: Nodes[] | null = null;
  for (const [index, child] of children.entries()) {
    const parts = child.type === "text" ? splitBareUrlsInText(child.value) : null;
    if (parts) {
      split ??= children.slice(0, index);
      split.push(...parts.map(part => ({ type: "text" as const, value: part })));
    } else {
      split?.push(child);
      splitBareUrls(child);
    }
  }
  if (split) parent.children = split as typeof parent.children;
}
