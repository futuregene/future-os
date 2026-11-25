/**
 * `<futureos-choice>` written as tags instead of a fence.
 *
 * The card's one spelling is a ` ```futureos-choice ` fence (`agent/src/prompt/choice.md`,
 * design `docs/design/choice-cards.md` §3). A model sometimes reaches for an
 * XML-ish wrapper instead —
 *
 *     <futureos-choice>
 *     title: 这两处要现在改吗？
 *     option: parser | 解析器和徽章都接受标签形式
 *     </futureos-choice>
 *
 * — and CommonMark has no idea what that is: the tags stay in the text, the
 * `option:` lines are swallowed with them, and the reader gets the card's source
 * run together instead of a card. Measured on two consecutive replies from one
 * model (`future/deepseek-flash`, 2026-11-24 and 2026-11-25): three of them wrote
 * the tag form for the card while quoting the fence form correctly in their prose.
 *
 * So a *closed* pair is read as the fence it stands for, and then goes through
 * the one parser (`parseChoiceCard`) like every other card: the field rules, the
 * option bounds and the "last card" rule are not written twice.
 *
 * Three mdast shapes have to be recognised, because the shape depends on what
 * surrounds the tags: a tag block that starts the document (or follows a blank
 * line with nothing after it) becomes an `html` node holding the raw lines up to
 * EOF; one that cannot interrupt what precedes it stays a `paragraph`, either
 * leading it or *sharing* it with the prose line just before (the 2026-11-25
 * reply wrote `…再问一遍：` then the tag on the next line, no blank line between —
 * a fence interrupts a paragraph there, so the tag form is the same attempt and
 * is split off the paragraph rather than printed as source).
 *
 * What is not accepted:
 *  - an unpaired tag, or the bare word `futureos-choice`, which stays the text
 *    it is (the reply prints it, and the agent warns — see `choice_card.rs`) —
 *    with one exception: an *opening* tag that ends the reply with a body of
 *    option rows. There is nothing left for it to swallow, and a writer that
 *    forgot the closer meant a card;
 *  - a pair inside another fence, which is a *quote* of the tag form, not an
 *    attempt at a card;
 *  - a body carrying a fence line, which would make the two readers disagree.
 */

import type { Code, Paragraph, PhrasingContent, RootContent } from "mdast";
import { readFenceLine } from "./fenceNesting";

/** The fence language in its one canonical spelling. */
export const CHOICE_FENCE = "futureos-choice";
const CHOICE_LANGUAGE = "futureoschoice";
const OPEN_TAG = "<futureos-choice>";
const CLOSE_TAG = "</futureos-choice>";
/** Rows a body needs before an *unclosed* tag is read as a card attempt. */
const UNCLOSED_MIN_ROWS = 2;

/**
 * Is `info` the card's fence language?
 *
 * Case, surrounding space and the `-`/`_` between the two words are the ways a
 * writer differs without meaning anything different (`FutureOS-Choice`,
 * `futureos_choice`): they are folded away here so the renderer and the agent's
 * badge (`choice_card.rs`) read one block the same way. Anything else still has
 * to match — `futureos-choice-x` is some other language, and guessing there
 * would draw a card out of a sample. A *space* is not one of them: CommonMark
 * splits the info string there, so `futureos choice` never arrives as one word.
 */
export function isChoiceFence(info: string | null | undefined): boolean {
  if (!info) return false;
  return info.trim().toLowerCase().replace(/[-_]+/g, "") === CHOICE_LANGUAGE;
}

function isTagLine(line: string, tag: string): boolean {
  return line.trim().toLowerCase() === tag;
}

/** A bare fence run carrying nothing: the closer the writer meant to emit. */
function isBareFenceLine(line: string): boolean {
  const fence = readFenceLine(line);
  return fence !== null && fence.info === "";
}

/** Does this line belong to a card body rather than to prose? */
function isOptionRow(line: string): boolean {
  return /^option\s*:/i.test(line.trim());
}

/**
 * Is the line one of the card's own fields (`title:`, `mode:`, `context:`,
 * `hint:`, `option:`) or blank?
 *
 * The unclosed-tag case needs it: with nothing after the tag to mark the end of
 * the card, a body of *prose* is the tag the reply prints, and reading it as a
 * body would swallow the sentence that follows it.
 */
function isCardField(line: string): boolean {
  return line.trim() === "" || /^(title|mode|context|hint|option)\s*:/i.test(line.trim());
}

/**
 * The lines of the pair that opens at `lines[open]`.
 *
 * A pair is its tags and nothing else: text after the closing tag would be
 * content this conversion drops, so only a bare fence run (the same malformed
 * emission's attempt at a closer) may follow it. `allowUnclosed` is the
 * reply's last block, where the missing closer has nothing left to swallow and
 * the body still has to look like a card (`UNCLOSED_MIN_ROWS`).
 */
function pairBody(lines: string[], open: number, allowUnclosed: boolean): string[] | null {
  if (open < 0 || open >= lines.length) return null;
  if (!isTagLine(lines[open]!, OPEN_TAG)) return null;
  const close = lines.findIndex((line, index) => index > open && isTagLine(line, CLOSE_TAG));

  let body: string[];
  if (close === -1) {
    if (!allowUnclosed) return null;
    body = lines.slice(open + 1);
  } else {
    const rest = lines.slice(close + 1).filter(line => line.trim() !== "");
    if (rest.length > 0 && !(rest.length === 1 && isBareFenceLine(rest[0]!))) return null;
    body = lines.slice(open + 1, close);
  }
  // A fence or a nested tag inside the body is not a card body; leaving the
  // block as written keeps the two readers (this one and `choice_card.rs`)
  // looking at the same shape.
  if (body.some(line => readFenceLine(line) !== null)) return null;
  if (body.some(line => line.toLowerCase().includes("futureos-choice"))) return null;
  if (close === -1 && body.filter(isOptionRow).length < UNCLOSED_MIN_ROWS) return null;
  if (close === -1 && !body.every(isCardField)) return null;
  return body;
}

/**
 * The mdast node as the raw lines it was written from.
 *
 * A paragraph's body is read from `raw` through the node's own offsets rather
 * than from the converted children: remark has already merged the soft line
 * breaks into spaces, and the directive lines need their real boundaries. An
 * `html` node already carries its raw lines in `value`.
 */
function nodeLines(node: RootContent, raw: string): string[] | null {
  if (node.type === "html") return node.value.split("\n");
  if (node.type !== "paragraph") return null;
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  if (start === undefined || end === undefined) return null;
  return raw.slice(start, end).split("\n");
}

/**
 * The paragraph a shared pair leaves behind: its inline children, cut where the
 * tag line begins.
 *
 * The children are kept rather than rebuilt from `raw` so the prose keeps its
 * marks (`**bold**`, a link) — a paragraph is split here, not rewritten. A child
 * that straddles the cut is kept as far as it goes (text) or dropped (anything
 * whose remainder would be a half-constructed node); `null` means the cut is not
 * safe to make, so the block stays the text it is.
 */
function proseBefore(node: Paragraph, splitOffset: number): Paragraph | null {
  const children: PhrasingContent[] = [];
  for (const child of node.children) {
    const start = child.position?.start.offset;
    const end = child.position?.end.offset;
    if (start === undefined || end === undefined) return null;
    if (end <= splitOffset) {
      children.push(child);
      continue;
    }
    if (child.type === "text" && start < splitOffset) {
      const value = child.value.slice(0, splitOffset - start);
      if (value) children.push({ type: "text", value });
    }
    break;
  }
  if (children.length === 0) return null;
  return { children, position: node.position, type: "paragraph" };
}

/**
 * `node` as the nodes it stands for: the `futureos-choice` code node a tag pair
 * means, preceded by the prose it shared a paragraph with — or `null` when the
 * node holds no pair.
 */
export function choiceTagBlocks(
  node: RootContent,
  raw: string,
  isLastBlock: boolean,
): RootContent[] | null {
  const lines = nodeLines(node, raw);
  if (!lines) return null;
  const open = lines.findIndex(line => isTagLine(line, OPEN_TAG));
  if (open === -1) return null;
  const body = pairBody(lines, open, isLastBlock);
  if (!body) return null;

  const code: Code = {
    lang: CHOICE_FENCE,
    position: node.position,
    type: "code",
    value: body.join("\n"),
  };
  if (open === 0 || node.type !== "paragraph") return [code];

  const start = node.position?.start.offset;
  if (start === undefined) return null;
  let splitOffset = start;
  for (let index = 0; index < open; index += 1) splitOffset += lines[index]!.length + 1;
  const prose = proseBefore(node, splitOffset);
  // A shared paragraph whose prose cannot be cut safely is left as written:
  // dropping the tag line would silently delete the reader's sentence.
  return prose ? [prose, code] : null;
}

/** A bare fence run carrying nothing: the closer the writer meant to emit. */
function isAttemptedCloser(node: RootContent, raw: string): boolean {
  if (node.type !== "code" || node.lang || node.value.trim() !== "") return false;
  const start = node.position?.start.offset;
  if (start === undefined) return false;
  const lineEnd = raw.indexOf("\n", start);
  const line = raw.slice(start, lineEnd === -1 ? undefined : lineEnd);
  return isBareFenceLine(line);
}

/**
 * `children` with every closed tag pair replaced by its fence's code node.
 *
 * An empty fence block immediately after a converted pair is the same malformed
 * emission's attempt at a closing fence (the 2026-11-24 reply ended
 * `</futureos-choice>` then ` ``` `): it holds nothing, so it is dropped and the
 * card stays the reply's last block — otherwise the card would render read-only
 * behind an empty code block, which is "it shows" rather than "it works".
 */
export function normalizeChoiceTagBlocks(children: RootContent[], raw: string): RootContent[] {
  const out: RootContent[] = [];
  for (let index = 0; index < children.length; index += 1) {
    const child = children[index]!;
    const blocks = choiceTagBlocks(child, raw, index === children.length - 1);
    if (!blocks) {
      out.push(child);
      continue;
    }
    out.push(...blocks);
    const next = children[index + 1];
    if (next && isAttemptedCloser(next, raw)) index += 1;
  }
  return out;
}
