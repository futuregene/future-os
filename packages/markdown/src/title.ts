/**
 * The title a conversation is shown by when it has no name of its own.
 *
 * An unnamed conversation is named after its first user message, and that
 * message is markdown *source*: a composer writes a reference to another
 * conversation as `[手机截图镜像修复](futureos://session/<id>)`, and a first
 * message beginning with one used to name its conversation
 * `[手机截图镜像修复](futureos://sessio…` — brackets, a URL, no topic. A title
 * is a place for text, not for drawing markdown, and the Desktop mints the
 * placeholder title itself (before any Agent session exists) from the raw
 * composer text.
 *
 * The rule mirrors `plain_title_text` in `packages/rpc/src/title.rs`, which is
 * what the Agent, the host's prompt path and the importer apply. The two
 * implementations must agree: the same message must not name a conversation one
 * way on the desktop and another way on the phone.
 */

/** The URL prefix that marks a link as a reference to another conversation. */
const SESSION_REFERENCE_URL = "futureos://session/";

interface InlineLink {
  /** Index just past the closing `)`. */
  end: number;
  label: string;
  target: string;
}

/**
 * Plain text of a user message, for a title or list preview derived from it.
 *
 * Three transformations, in order:
 *
 * 1. A reference to another conversation is *addressing*, not prose, so it
 *    contributes nothing. Its label is the other conversation's title, and
 *    keeping it named this conversation after that one — two identical rows in
 *    one list, where the reader cannot tell which one they are looking at.
 * 2. Every other link contributes its label and an image its alt text, so the
 *    title reads as the sentence that was written.
 * 3. Markdown syntax and composer placeholders are removed and the text that is
 *    left is kept — see {@link flattenMarkup}.
 *
 * A message that is *only* a reference keeps the label after all: naming a
 * conversation after the one it points at beats naming it `[`.
 */
export function plainTitleText(text: string): string {
  const withoutReferences = flattenMarkup(renderTitleText(text, true));
  if (withoutReferences) return withoutReferences;
  // Nothing but a reference (or nothing at all): the labels are all we have.
  return flattenMarkup(renderTitleText(text, false));
}

/**
 * Markdown source read out as plain text: the words, none of the syntax.
 *
 * A title is drawn as text, so nothing that only exists to be rendered may
 * survive in it. That covers two kinds of leftovers:
 *
 * - **Composer placeholders**, which name nothing a reader can use: the TUI
 *   writes `[Image #1]` where a pasted path became an attachment, and folds a
 *   long paste into `[Pasted Content 2000 chars]`. A message that was one
 *   screenshot used to name its conversation `[Image #1][Image #2]`.
 * - **Inline and block syntax**: emphasis, strong, strikethrough, code spans
 *   and fences, headings, block quotes, list bullets and ordered-list numbers.
 *   The text inside them stays; only the marks go, so `**加粗** 一下` reads
 *   `加粗 一下`.
 *
 * Delimiters are dropped only when they pair up, because a title may not eat
 * text that merely contains one: `2 * 3` and `a_b_c` keep their characters, and
 * an unclosed `**` stays as written. Whitespace is collapsed, because a title is
 * one line whatever the message's line breaks were.
 */
export function flattenMarkup(text: string): string {
  const kept = withoutPlaceholders(text);
  const lines = withoutBlockMarkers(kept);
  const plain = withoutDelimiters(lines);
  return plain.replace(/\s+/g, " ").trim();
}

/**
 * Length of a composer placeholder at the start of `text`, or 0 when there is
 * none. Recognised exactly as the composers write them — `[Image #<n>]`,
 * `[Pasted Content <n> chars]` and `[Pasted Content <n> chars #<n>]` — so
 * ordinary bracketed prose (`[pending] 待办`) is never mistaken for one.
 */
function placeholderLength(text: string): number {
  if (!text.startsWith("[")) return 0;
  const image = /^\[Image #\d+\]/.exec(text);
  if (image) return image[0].length;
  const paste = /^\[Pasted Content \d+ chars(?: #\d+)?\]/.exec(text);
  if (paste) return paste[0].length;
  return 0;
}

/** Drop every composer placeholder, leaving the prose around it. */
function withoutPlaceholders(text: string): string {
  let kept = "";
  let index = 0;
  while (index < text.length) {
    if (text[index] === "[") {
      const length = placeholderLength(text.slice(index, index + 48));
      if (length > 0) {
        index += length;
        continue;
      }
    }
    kept += text[index];
    index += 1;
  }
  return kept;
}

/** Drop the marks that only exist to open a block, keeping each line's text. */
function withoutBlockMarkers(text: string): string {
  return text
    .split("\n")
    .map((line) => (isRuleOrFence(line) ? "" : line.slice(blockMarkerLength(line))))
    .join("\n");
}

/**
 * A line that carries no text of its own: a horizontal rule, a setext
 * underline or a code fence.
 */
function isRuleOrFence(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  const fences = /^[`~]{3,}/.exec(trimmed);
  if (fences) return fences[0].split("").every((ch) => ch === trimmed[0]);
  const marks = trimmed.replace(/\s+/g, "");
  return (
    marks.length >= 3 &&
    /^([-*_=])\1*$/.test(marks) &&
    marks.split("").every((ch) => ch === marks[0])
  );
}

/** How many characters of `line` only open a block and are not text. */
function blockMarkerLength(line: string): number {
  const indent = /^\s*/.exec(line)![0].length;
  const rest = line.slice(indent);
  const heading = /^#{1,6}(?: |$)/.exec(rest);
  if (heading) return indent + heading[0].length;
  if (rest.startsWith(">")) {
    let quote = 0;
    while (rest[quote] === ">") {
      quote += 1;
      if (rest[quote] === " ") quote += 1;
    }
    return indent + quote;
  }
  if (/^[-*+] /.test(rest)) return indent + 2;
  const ordered = /^\d+[.)] /.exec(rest);
  if (ordered) return indent + ordered[0].length;
  return 0;
}

/**
 * Drop emphasis, strong, strikethrough and code delimiters that pair up.
 *
 * Only paired delimiters go: `**b**` keeps `b`, while `2 * 3` and `a_b_c` keep
 * their characters, and an unclosed `**` stays as it was written.
 */
function withoutDelimiters(text: string): string {
  const dropped = new Array<boolean>(text.length).fill(false);
  let index = 0;
  while (index < text.length) {
    const ch = text[index];
    if (ch !== "*" && ch !== "_" && ch !== "~" && ch !== "`") {
      index += 1;
      continue;
    }
    const run = runLength(text, index);
    if (opensEmphasis(text, index, run)) {
      const close = closingDelimiter(text, index + run, ch, run);
      if (close >= 0) {
        const closeRun = runLength(text, close);
        dropped.fill(true, index, index + run);
        dropped.fill(true, close, close + closeRun);
        index += run;
        continue;
      }
    }
    index += run;
  }
  let kept = "";
  for (let at = 0; at < text.length; at += 1) if (!dropped[at]) kept += text[at];
  return kept;
}

function runLength(text: string, from: number): number {
  let end = from;
  while (end < text.length && text[end] === text[from]) end += 1;
  return end - from;
}

const WORD = /[\p{L}\p{N}]/u;

function opensEmphasis(text: string, from: number, run: number): boolean {
  const ch = text[from];
  // `_` inside a word is a name (`a_b_c`), not emphasis; for every delimiter the
  // mark has to be followed by the text it marks.
  const intraword = ch === "_" && from > 0 && WORD.test(text[from - 1]!);
  const content = from + run < text.length && !/\s/.test(text[from + run]!);
  return !intraword && content;
}

function closingDelimiter(text: string, from: number, ch: string, run: number): number {
  let index = from;
  while (index < text.length) {
    if (text[index] !== ch) {
      index += 1;
      continue;
    }
    const length = runLength(text, index);
    const closes = index > 0 && !/\s/.test(text[index - 1]!);
    const intraword =
      ch === "_" && index + length < text.length && WORD.test(text[index + length]!);
    if (closes && !intraword && length >= run) return index;
    index += length;
  }
  return -1;
}

/** Whether a link target addresses another conversation in this app. */
function isConversationReference(target: string): boolean {
  // Both spellings of a target are in the wild: `[a](x)` and `[a](<x>)`.
  const trimmed = target.trim();
  const bare = trimmed.startsWith("<") && trimmed.endsWith(">")
    ? trimmed.slice(1, -1)
    : trimmed;
  return bare.startsWith(SESSION_REFERENCE_URL);
}

/** `dropReferences` decides what a conversation reference contributes: its label, or nothing. */
function renderTitleText(text: string, dropReferences: boolean): string {
  let rendered = "";
  let copied = 0;
  let index = 0;
  while (index < text.length) {
    const link = inlineLinkAt(text, index);
    if (!link) {
      index += 1;
      continue;
    }
    rendered += text.slice(copied, index);
    if (!(dropReferences && isConversationReference(link.target)))
      rendered += link.label;
    index = link.end;
    copied = index;
  }
  rendered += text.slice(copied);
  // Line structure survives: a link never spans a line, and `flattenMarkup`
  // needs the message's lines to tell a heading or a bullet from prose.
  return rendered;
}

/**
 * An inline markdown image (`![alt](target)`) or link (`[label](target)`)
 * starting at `from`, or null when no *complete* link starts there — text that
 * merely contains brackets is left exactly as it was, because a title may not
 * silently eat it.
 */
function inlineLinkAt(text: string, from: number): InlineLink | null {
  let start = from;
  if (text[start] === "!") start += 1;
  if (text[start] !== "[") return null;
  const labelStart = start + 1;
  const labelEnd = text.indexOf("]", labelStart);
  if (labelEnd < 0) return null;
  if (text[labelEnd + 1] !== "(") return null;
  // Targets can carry parentheses of their own (`[x](file(1).md)`), so the
  // link ends at the `)` that closes the one that opened it.
  const targetStart = labelEnd + 2;
  let depth = 1;
  let end = targetStart;
  for (; end < text.length; end += 1) {
    if (text[end] === "(") depth += 1;
    else if (text[end] === ")") {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  if (depth !== 0) return null; // unterminated — not a link
  return {
    end: end + 1,
    label: text.slice(labelStart, labelEnd),
    target: text.slice(targetStart, end),
  };
}
