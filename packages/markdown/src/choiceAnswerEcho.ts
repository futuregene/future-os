import type { FenceLine } from "./fenceNesting";
import { closesFence, readFenceLine } from "./fenceNesting";

/**
 * The answer to a `futureos-choice` card, read back out of a sent prompt.
 *
 * Sending writes plain text — `[接下来做哪一块？ → 把归因搬进页面]` — on purpose
 * (design `docs/design/choice-cards.zh-CN.md` §7): the answer has to survive the
 * paths that never drew the card, a TUI, an IM bot, a paste elsewhere, so it
 * cannot be a `futureos://` scheme only one client understands.
 *
 * That leaves the transcript showing the sentence as the reader's own prose,
 * where a bracket sandwich reads as something they typed rather than as a pick
 * they made. This reads the answers back so a bubble can draw them as tags. It
 * only *classifies*, and takes those runs out of the prose it returns: the
 * prompt is not rewritten, and what the copy button yields is still the sentence
 * that was sent.
 *
 * The answer is not necessarily at the head. The composer appends its pill to
 * the end of the editor (`writeChoicePill`), so a reader who types first and
 * picks a row afterwards sends `另外 asrbench 可以删除[删哪些？ → 删未在用的 ~/omlx 副本]`
 * — the answer sits in the middle of their own sentence, and drawing it as a tag
 * is no less right there than at the head.
 */

/** One printed answer: the card's question and the rows that were picked. */
export interface ChoiceAnswerEcho {
  /** The rows as the answer joined them (`、`) — what the card marked. */
  answer: string;
  /** The question the card asked. */
  question: string;
}

export interface ChoiceAnswerEchoSplit {
  /** The answers, in the order the prompt printed them. */
  echoes: ChoiceAnswerEcho[];
  /** What the reader typed around them, in order, with the answers taken out. */
  rest: string;
}

/**
 * `[question → answer]`, which is exactly what `choiceAnswerText` writes.
 *
 * Its two halves exclude brackets and newlines so a match cannot swallow the
 * next answer. Nothing anchors it to the head of the text: the pill is appended
 * to the end of the editor, so a mid-sentence answer is the normal shape of a
 * prompt that was partly typed before a row was picked.
 */
const ANSWER = /\[(\S[^[\]\n]{0,199}?)[ \t]*→[ \t]*(\S[^[\]\n]{0,299}?)\]/gu;

/**
 * A card holds at most four options (design §3), so a longer run of bracketed
 * arrows is prose that happens to look like an answer. Reading four is already
 * more than a draft can hold; the bound only stops a weird message from being
 * walked to its end.
 */
const MAX_ECHOES = 4;

/** One half-open range of the prompt that is code, not prose. */
interface SourceRange {
  end: number;
  start: number;
}

/**
 * Split a sent prompt into the answers it prints and the text around them.
 *
 * Conservative on purpose: a bracket run must carry the arrow the printer writes
 * and have both halves non-empty, one followed by `(` or `[` is a link or
 * another bracket construct, and a run inside code — a fenced block or an inline
 * span — is an example the reader pasted, not a pick. A false negative costs
 * only the styling.
 */
export function splitChoiceAnswerEcho(text: string): ChoiceAnswerEchoSplit {
  const code = codeRanges(text);
  const echoes: ChoiceAnswerEcho[] = [];
  const cuts: SourceRange[] = [];
  ANSWER.lastIndex = 0;
  for (
    let match = ANSWER.exec(text);
    match && echoes.length < MAX_ECHOES;
    match = ANSWER.exec(text)
  ) {
    const start = match.index;
    const end = start + match[0].length;
    if (overlaps(code, start, end))
      continue;
    const after = text.charAt(end);
    if (after === "(" || after === "[")
      continue;
    echoes.push({ answer: match[2]!.trim(), question: match[1]!.trim() });
    cuts.push({ end, start });
  }
  // No answer: hand the caller the string it passed in, not a copy whose
  // whitespace it already normalized away.
  if (echoes.length === 0)
    return { echoes, rest: text };
  return { echoes, rest: withoutRanges(text, cuts) };
}

/**
 * The prompt with the answers taken out.
 *
 * A cut closes over the whitespace it was sitting in — `先说 [q → a] 再说` must
 * not leave a double space, and a run that had a line to itself must not leave a
 * blank one — which is the same tidying the head case always did, now applied at
 * every seam.
 */
function withoutRanges(text: string, cuts: readonly SourceRange[]): string {
  let rest = "";
  let cursor = 0;
  for (const cut of cuts) {
    rest = appendChunk(rest, text.slice(cursor, cut.start));
    cursor = cut.end;
  }
  return appendChunk(rest, text.slice(cursor)).trimEnd();
}

function appendChunk(rest: string, chunk: string): string {
  if (rest === "" || /\s$/u.test(rest))
    return rest + chunk.replace(/^\s+/u, "");
  return rest + chunk;
}

/**
 * Every range of the prompt that renders as code.
 *
 * The classifier reads the source, not the tree, so nothing stops it from
 * finding an arrow inside a fenced block or a pair of backticks. Cutting one out
 * would leave the fence or the backticks unbalanced and corrupt the markdown
 * around it, so those ranges are excluded up front — a pasted example keeps its
 * brackets.
 */
function codeRanges(text: string): SourceRange[] {
  const ranges: SourceRange[] = [];
  let fence: FenceLine | null = null;
  let fenceStart = 0;
  let offset = 0;
  for (const line of text.split("\n")) {
    const start = offset;
    const end = offset + line.length;
    offset = end + 1;
    if (fence) {
      if (closesFence(line, fence)) {
        ranges.push({ end, start: fenceStart });
        fence = null;
      }
      continue;
    }
    const open = readFenceLine(line);
    if (open) {
      fence = open;
      fenceStart = start;
      continue;
    }
    ranges.push(...codeSpanRanges(line, start));
  }
  // An unclosed fence runs to the end of the prompt — that is what a fence that
  // is still being typed means.
  if (fence)
    ranges.push({ end: text.length, start: fenceStart });
  return ranges;
}

/**
 * The inline code spans of one line, in source offsets.
 *
 * A backtick run pairs with the next run of the *same* length (CommonMark); one
 * with no partner is literal text, so a lone backtick in prose does not hide the
 * rest of the line.
 */
function codeSpanRanges(line: string, base: number): SourceRange[] {
  const ranges: SourceRange[] = [];
  let index = 0;
  while (index < line.length) {
    if (line[index] !== "`") {
      index += 1;
      continue;
    }
    const open = index;
    const openLength = runLength(line, open);
    let probe = open + openLength;
    let close = -1;
    while (probe < line.length) {
      if (line[probe] !== "`") {
        probe += 1;
        continue;
      }
      const length = runLength(line, probe);
      if (length === openLength) {
        close = probe;
        break;
      }
      probe += length;
    }
    if (close < 0) {
      index = open + openLength;
      continue;
    }
    const end = close + openLength;
    ranges.push({ end: base + end, start: base + open });
    index = end;
  }
  return ranges;
}

/** Length of the backtick run starting at `index`. */
function runLength(line: string, index: number): number {
  let length = 0;
  while (line[index + length] === "`") length += 1;
  return length;
}

function overlaps(ranges: readonly SourceRange[], start: number, end: number): boolean {
  return ranges.some(range => start < range.end && range.start < end);
}
