import type { ChoiceCardMode } from "../../markdown/futureMarkdownTypes";

/** One option as the answer carries it: enough to print and to re-offer it. */
export interface ChoiceAnswerOption {
  id: string;
  label: string;
}

/**
 * The answer a `futureos-choice` card holds, as the composer's editor stores it.
 *
 * One shape for the three places it appears (design
 * `docs/design/choice-cards.zh-CN.md` §2): the card's marked row, the pill in
 * the editor, and the text that is sent. It is deliberately plain data, and it
 * carries **all** of the card's options — not just the picked ones — so the
 * pill's pencil can re-offer the others from the DOM alone, without asking the
 * parent for the card again.
 */
export interface ChoiceAnswer {
  mode: ChoiceCardMode;
  /** Every option of the card, in card order. */
  options: ChoiceAnswerOption[];
  /** The picked options, in the order they were picked. */
  selectedIds: string[];
  /** The card's question, so the sent text attributes the answer (§7). */
  title: string;
}

/**
 * A card as a pick hands it on: the question it asks and every row it offers,
 * without a pick. This is what `MarkdownContent`'s `choiceAnswer.onSelect` adds
 * to the clicked option id, and it is how an answer gets its question the first
 * time a row is clicked (before that the draft holds no answer to read it from).
 *
 * Structural on purpose: the card's own `ChoiceCardOption` carries a
 * `description` the answer does not keep (the pill prints labels, and the
 * popover deliberately drops the explanations once they have been read — §5.3).
 */
export type ChoiceCardQuestion = Pick<ChoiceAnswer, "mode" | "options" | "title">;

/**
 * Turn a click on one of the card's rows into the answer the draft keeps.
 *
 * `previous` is whatever the draft already held; a pick from a *different*
 * question starts over rather than merging, because an answer belongs to one
 * card and a title left over from the previous one would be printed on send.
 * Same question, same rows → the pick applies to it (single replaces, multi
 * toggles, `pickChoiceOption`).
 */
export function answerFromPick(
  previous: ChoiceAnswer | null,
  card: ChoiceCardQuestion,
  optionId: string,
): ChoiceAnswer {
  const sameQuestion = previous !== null
    && previous.title === card.title
    && previous.mode === card.mode;
  const base: ChoiceAnswer = sameQuestion
    ? previous
    : {
        mode: card.mode,
        options: card.options.map(option => ({ id: option.id, label: option.label })),
        selectedIds: [],
        title: card.title,
      };
  return pickChoiceOption(base, optionId);
}

/** Labels of the picked options, in order. */
function selectedLabels(answer: ChoiceAnswer): string[] {
  return answer.selectedIds
    .map(id => answer.options.find(option => option.id === id)?.label)
    .filter((label): label is string => Boolean(label));
}

/** What the pill prints — a multi-choice answer reads `重画森林图、补一张流程图`. */
export function choiceAnswerLabel(answer: ChoiceAnswer): string {
  return selectedLabels(answer).join("、");
}

/**
 * What sending writes (§7): `[接下来做哪一件？ → 重画森林图]`.
 *
 * Plain text on purpose. The answer has to survive the degraded paths — TUI, an
 * IM bot, a message pasted elsewhere — so it must be a sentence any client can
 * read, not a `futureos://` scheme that only the desktop understands.
 */
export function choiceAnswerText(answer: ChoiceAnswer): string {
  return `[${answer.title} → ${choiceAnswerLabel(answer)}]`;
}

/** Turn a pick into the answer to send: single replaces, multi toggles (§6 b). */
export function pickChoiceOption(answer: ChoiceAnswer, optionId: string): ChoiceAnswer {
  if (answer.mode === "multi") {
    const picked = answer.selectedIds.includes(optionId)
      ? answer.selectedIds.filter(id => id !== optionId)
      : [...answer.selectedIds, optionId];
    return { ...answer, selectedIds: picked };
  }
  return { ...answer, selectedIds: [optionId] };
}

/**
 * The answer a draft should hold, given what a pick produced.
 *
 * A pill (and the phone's chip) exists to answer *what did the reader pick*. Zero
 * ticks answers nothing, so the answer is dropped instead of kept as a question
 * with no pick: a pill built from it would print the question alone and claim the
 * reader had answered it. §7 already says zero ticks send no tag; this is the same
 * rule one layer up, so no surface has to repeat it.
 *
 * Every place an answer is *written* goes through here (`MentionEditor`'s pill,
 * `Composer`/`RemoteComposer`'s state, the phone's draft). `pickChoiceOption` and
 * the other pure helpers keep their signatures: they describe a pick, this decides
 * whether the draft has an answer at all.
 */
export function answerOrNull(answer: ChoiceAnswer | null): ChoiceAnswer | null {
  return answer && answer.selectedIds.length > 0 ? answer : null;
}

/** Read back a pill's `data-choice`; malformed data yields `null`, never a throw. */
export function parseChoiceAnswer(raw: string | null): ChoiceAnswer | null {
  if (!raw)
    return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object")
      return null;
    const { mode, options, selectedIds, title } = parsed as Partial<ChoiceAnswer>;
    if (typeof title !== "string" || !title.trim())
      return null;
    if (mode !== "single" && mode !== "multi")
      return null;
    if (!Array.isArray(options) || options.length === 0 || !Array.isArray(selectedIds))
      return null;
    const validOptions = options.every(
      option => option && typeof option.id === "string" && typeof option.label === "string",
    );
    if (!validOptions || !selectedIds.every(id => typeof id === "string"))
      return null;
    return { mode, options: options as ChoiceAnswerOption[], selectedIds: selectedIds as string[], title };
  }
  catch {
    return null;
  }
}
