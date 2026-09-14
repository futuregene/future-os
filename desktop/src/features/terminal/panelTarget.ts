/**
 * Which conversation (if any) currently offers a terminal panel.
 *
 * Extracted from `AppShell` so the rule is unit-testable, and because getting it
 * wrong is invisible: the panel simply never appears, which reads as "the
 * feature was never implemented" rather than as a bug.
 *
 * Both thread sections count. Selecting a workspace conversation sets the
 * shell's section to `"workspace"`, not `"chat"` — an earlier version gated on
 * `"chat"` alone and hid the entry point for every workspace conversation,
 * which is the common case.
 */
export function terminalTarget(input: {
  section: string;
  centerMode: string;
  threadId: string | null | undefined;
}): string | null {
  if (input.centerMode !== "thread")
    return null;
  if (input.section !== "chat" && input.section !== "workspace")
    return null;
  return input.threadId ?? null;
}
