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
 * they made. This reads the leading answers back so a bubble can draw them as a
 * tag. It only *classifies* the characters: the prompt is not rewritten, and
 * what the copy button yields is still the sentence that was sent.
 */

/** One printed answer: the card's question and the rows that were picked. */
export interface ChoiceAnswerEcho {
  /** The rows as the answer joined them (`、`) — what the card marked. */
  answer: string;
  /** The question the card asked. */
  question: string;
}

export interface ChoiceAnswerEchoSplit {
  /** Leading answers in send order; empty when the text prints none. */
  echoes: ChoiceAnswerEcho[];
  /** Everything after them, verbatim — what the reader typed themselves. */
  rest: string;
}

/**
 * `[question → answer]`, which is exactly what `choiceAnswerText` writes.
 *
 * Its two halves exclude brackets and newlines so a match cannot swallow the
 * next answer, and the run itself is anchored to the head of the text: an
 * answer is printed first, before anything the reader adds after it (§5.2.2).
 */
const ANSWER = /^\[(\S[^[\]\n]{0,199}?)[ \t]*→[ \t]*(\S[^[\]\n]{0,299}?)\]/u;

/**
 * A card holds at most four options (design §3), so a longer run of bracketed
 * arrows is prose that happens to look like an answer. Reading four is already
 * more than a draft can hold; the bound only stops a weird message from being
 * walked to its end.
 */
const MAX_ECHOES = 4;

/**
 * Split a sent prompt into the answers its head prints and the text after them.
 *
 * Conservative on purpose: a prompt that merely *starts* with bracketed text is
 * left alone unless it carries the arrow the printer writes, and a bracket run
 * followed by `(` or `[` keeps its markdown — that is a link or a second bracket
 * construct, not an answer. A false negative costs only the styling.
 */
export function splitChoiceAnswerEcho(text: string): ChoiceAnswerEchoSplit {
  const echoes: ChoiceAnswerEcho[] = [];
  let rest = text;
  while (echoes.length < MAX_ECHOES) {
    const match = ANSWER.exec(rest);
    if (!match)
      break;
    const after = rest.charAt(match[0].length);
    if (after === "(" || after === "[")
      break;
    echoes.push({ answer: match[2]!.trim(), question: match[1]!.trim() });
    rest = rest.slice(match[0].length).replace(/^\s+/u, "");
  }
  // No answer at the head: hand the caller the string it passed in, not a copy
  // whose whitespace it already normalized away.
  if (echoes.length === 0)
    return { echoes, rest: text };
  return { echoes, rest };
}
