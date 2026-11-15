import type { ChoiceCardMode, ChoiceCardNode } from "@future-os/markdown";

/** One option as an answer carries it: enough to print and to re-offer it. */
export interface ChoiceAnswerOption {
  id: string;
  label: string;
}

/**
 * The answer a `futureos-choice` card holds, as this phone's composer draft
 * keeps it.
 *
 * One shape for the three places it appears (design
 * `docs/design/choice-cards.zh-CN.md` §2): the card's marked row, the chip above
 * the field, and the text that is sent. It carries **all** of the card's options
 * — not just the picked ones — so the chip's pencil can re-offer the others
 * without asking the transcript for the card again.
 *
 * The phone has no equivalent of the desktop's in-editor pill: an RN `TextInput`
 * is a native textarea with no atomic elements. So this value *is* the draft
 * item the composer counts (see `ComposerDock`'s `canSend`/`hasDraft`) rather
 * than a decal drawn beside it (§5.7.1).
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

/** The answer a picked row produces: the card's question and its whole choice. */
export function choiceAnswerFromCard(card: ChoiceCardNode): ChoiceAnswer {
  return {
    mode: card.mode,
    options: card.options.map(option => ({ id: option.id, label: option.label })),
    selectedIds: [],
    title: card.title,
  };
}

/** Labels of the picked options, in order. */
function selectedLabels(answer: ChoiceAnswer): string[] {
  return answer.selectedIds
    .map(id => answer.options.find(option => option.id === id)?.label)
    .filter((label): label is string => Boolean(label));
}

/** What the chip prints — a multi-choice answer reads `重画森林图、补一张流程图`. */
export function choiceAnswerLabel(answer: ChoiceAnswer): string {
  return selectedLabels(answer).join("、");
}

/**
 * What sending writes (§7): `[接下来做哪一件？ → 重画森林图]`.
 *
 * Plain text on purpose. The answer has to survive the degraded paths — the
 * desktop, a TUI, an IM bot, a message pasted elsewhere — so it is a sentence any
 * client can read, not a `futureos://` scheme only this phone understands.
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
 * A chip exists to answer *what did the reader pick*. Zero ticks answers nothing,
 * so the answer is dropped instead of kept as a question with no pick: a chip built
 * from it would print the question alone and claim the reader had answered it. §7
 * already says zero ticks send no tag; this is the same rule one layer up, so every
 * write to the draft goes through it.
 */
export function answerOrNull(answer: ChoiceAnswer | null): ChoiceAnswer | null {
  return answer && answer.selectedIds.length > 0 ? answer : null;
}

/**
 * Read a stored answer back, or `null`.
 *
 * A draft is restored from disk, so the value can be anything a previous version
 * wrote; a malformed one is dropped rather than half-restored — an answer that
 * cannot print its own question would send a sentence nobody asked for.
 */
export function parseChoiceAnswer(raw: unknown): ChoiceAnswer | null {
  if (!raw || typeof raw !== "object")
    return null;
  const { mode, options, selectedIds, title } = raw as Partial<ChoiceAnswer>;
  if (typeof title !== "string" || !title.trim())
    return null;
  if (mode !== "single" && mode !== "multi")
    return null;
  if (!Array.isArray(options) || options.length === 0 || !Array.isArray(selectedIds))
    return null;
  const optionsOk = options.every(option => option
    && typeof option.id === "string" && typeof option.label === "string");
  if (!optionsOk)
    return null;
  if (!selectedIds.every(id => typeof id === "string"))
    return null;
  const known = new Set((options as ChoiceAnswerOption[]).map(option => option.id));
  const picked = (selectedIds as string[]).filter(id => known.has(id));
  if (picked.length === 0)
    return null;
  return { mode, options: options as ChoiceAnswerOption[], selectedIds: picked, title };
}
