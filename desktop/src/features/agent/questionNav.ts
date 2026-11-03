/** Minimal shape needed to identify a row and its author. */
export interface TranscriptMessage {
  id: string;
  role: "assistant" | "system" | "tool" | "user";
}

/**
 * A row index in the rendered message list, from 0 (oldest) to length - 1
 * (newest). The desktop transcript is drawn in chronological order — unlike
 * mobile's inverted list — so a *larger* index is further down the page.
 */
export type MessageIndex = number;

/**
 * The rows the ↑/↓ pair is allowed to come to rest on: the reader's own
 * prompts, in transcript order.
 *
 * Answers are deliberately not stops. The pair is for finding what was *asked*
 * — skim a long conversation by its questions — so a press never lands in the
 * middle of a reply, however much of the page it takes. Both directions walk
 * this same list, one prompt per press.
 */
export function questionIndices(messages: readonly TranscriptMessage[]): MessageIndex[] {
  const indices: MessageIndex[] = [];
  messages.forEach((message, index) => {
    if (message.role === "user")
      indices.push(index);
  });
  return indices;
}

/**
 * The rung ↑ lands on: the last prompt entirely above the reader.
 *
 * `top` is the row the reader is reading (see `useQuestionNav`), so a prompt at
 * that row is not a destination — it is where they already are, and skipping it
 * is what makes the press after a landing move instead of re-aligning what is
 * already on screen.
 */
export function previousQuestion(
  questions: readonly MessageIndex[],
  top: MessageIndex | null,
): MessageIndex | null {
  if (top === null)
    return null;
  for (let position = questions.length - 1; position >= 0; position -= 1) {
    const index = questions[position]!;
    if (index < top)
      return index;
  }
  return null;
}

/**
 * The rung ↓ lands on: the first prompt below the reader.
 *
 * The row at the top edge is excluded — it is not below — which is what makes ↓
 * land on something new after a jump instead of re-selecting the row it landed
 * on. A press from inside an answer therefore reaches the next prompt past it,
 * and the answer itself is never a stop: that is the whole point of the pair,
 * not an exception to it.
 */
export function nextQuestion(
  questions: readonly MessageIndex[],
  top: MessageIndex | null,
): MessageIndex | null {
  if (top === null)
    return null;
  for (const index of questions) {
    if (index > top)
      return index;
  }
  return null;
}
