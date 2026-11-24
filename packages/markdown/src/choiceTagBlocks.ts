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
 * model (`future/deepseek-flash`, 2026-11-24): both wrote the tag form for the
 * card while quoting the fence form correctly in their prose.
 *
 * So a *closed* pair is read as the fence it stands for, and then goes through
 * the one parser (`parseChoiceCard`) like every other card: the field rules, the
 * option bounds and the "last card" rule are not written twice.
 *
 * Two mdast shapes have to be recognised, because the shape depends on what
 * surrounds the tags: a tag block that starts the document (or follows a blank
 * line with nothing after it) becomes an `html` node holding the raw lines up to
 * EOF, while one that cannot interrupt what precedes it stays a `paragraph`.
 *
 * What is not accepted:
 *  - an unpaired tag, or the bare word `futureos-choice`, which stays the text
 *    it is (the reply prints it, and the agent warns — see `choice_card.rs`);
 *  - a pair inside another fence, which is a *quote* of the tag form, not an
 *    attempt at a card;
 *  - a body carrying a fence line, which would make the two readers disagree.
 */

import type { Code, RootContent } from "mdast";
import { readFenceLine } from "./fenceNesting";

/** The fence language both the renderer and the agent's badge recognise. */
const CHOICE_FENCE = "futureos-choice";
const OPEN_TAG = "<futureos-choice>";
const CLOSE_TAG = "</futureos-choice>";

function isTagLine(line: string, tag: string): boolean {
  return line.trim().toLowerCase() === tag;
}

/** A bare fence run carrying nothing: the closer the writer meant to emit. */
function isBareFenceLine(line: string): boolean {
  const fence = readFenceLine(line);
  return fence !== null && fence.info === "";
}

/**
 * The lines between a pair of tags, or `null` when `lines` is not a closed pair
 * plus, at most, the empty fence run the writer meant as a closer.
 */
function pairBody(lines: string[]): string[] | null {
  if (lines.length < 3) return null;
  if (!isTagLine(lines[0]!, OPEN_TAG)) return null;
  const close = lines.findIndex((line, index) => index > 0 && isTagLine(line, CLOSE_TAG));
  if (close === -1) return null;
  // Nothing but the attempted closer may follow the closing tag: text there
  // would be content this conversion silently drops.
  const rest = lines.slice(close + 1).filter(line => line.trim() !== "");
  if (rest.length > 0 && !(rest.length === 1 && isBareFenceLine(rest[0]!))) return null;
  const body = lines.slice(1, close);
  // A fence or a nested tag inside the body is not a card body; leaving the
  // block as written keeps the two readers (this one and `choice_card.rs`)
  // looking at the same shape.
  if (body.some(line => readFenceLine(line) !== null)) return null;
  if (body.some(line => line.toLowerCase().includes("futureos-choice"))) return null;
  return body;
}

/**
 * `node` as the `futureos-choice` code node a tag pair stands for, or `null`
 * when the node is not a closed pair.
 *
 * A paragraph's body is read from `raw` through the node's own offsets rather
 * than from the converted children: remark has already merged the soft line
 * breaks into spaces, and the directive lines need their real boundaries. An
 * `html` node already carries its raw lines in `value`.
 */
export function choiceTagCode(node: RootContent, raw: string): Code | null {
  let lines: string[];
  if (node.type === "html") {
    lines = node.value.split("\n");
  } else if (node.type === "paragraph") {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    if (start === undefined || end === undefined) return null;
    lines = raw.slice(start, end).split("\n");
  } else {
    return null;
  }
  const body = pairBody(lines);
  if (!body) return null;
  return {
    lang: CHOICE_FENCE,
    position: node.position,
    type: "code",
    value: body.join("\n"),
  };
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
    const code = choiceTagCode(child, raw);
    if (!code) {
      out.push(child);
      continue;
    }
    out.push(code);
    const next = children[index + 1];
    if (next && isAttemptedCloser(next, raw)) index += 1;
  }
  return out;
}
