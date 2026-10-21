/**
 * Which conversation (if any) currently offers a terminal panel.
 *
 * Extracted from `AppShell` so the rule is unit-testable, and because getting it
 * wrong is invisible: the panel simply never appears, which reads as "the
 * feature was never implemented" rather than as a bug.
 *
 * The window has one conversation page, so the question is "is the conversation
 * column up?" rather than "which of the two conversation sections is up". It
 * used to name `chat` and `workspace` separately — two sections that answered one
 * question, and an earlier version that matched only `chat` hid the entry point
 * for every workspace conversation. The rail's tab decides *which list* is drawn
 * and has nothing to say about the terminal, which is why it is not read here.
 */
export function terminalTarget(input: {
  section: string;
  centerMode: string;
  threadId: string | null | undefined;
}): string | null {
  if (input.centerMode !== "thread")
    return null;
  if (input.section !== "conversations")
    return null;
  return input.threadId ?? null;
}
