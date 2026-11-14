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
