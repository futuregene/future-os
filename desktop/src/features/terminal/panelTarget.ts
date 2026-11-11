/**
 * Which conversation (if any) currently offers a terminal panel, and whose shell
 * it would be.
 *
 * Extracted from `AppShell` so the rule is unit-testable, and because getting it
 * wrong is invisible: the panel simply never appears, which reads as "the
 * feature was never implemented" rather than as a bug.
 *
 * Both thread sections count. Selecting a workspace conversation sets the
 * shell's section to `"workspace"`, not `"chat"` — an earlier version gated on
 * `"chat"` alone and hid the entry point for every workspace conversation,
 * which is the common case.
 *
 * A remote conversation takes precedence: its shells run on that machine, in
 * that conversation's directory, and the panel is scoped to the *host's* session
 * so the two machines can never adopt each other's tabs. It is offered only when
 * the host advertised the capability — a host that cannot open a shell answers
 * every terminal command with `Unsupported command`, so an entry point there
 * would be a button that only produces an error.
 */

export type TerminalScope
  = | { kind: "local"; threadId: string; key: string }
    | { kind: "remote"; desktopId: string; sessionId: string; key: string };

export interface TerminalScopeInput {
  section: string;
  centerMode: string;
  threadId: string | null | undefined;
  /**
   * The remote conversation on screen, if any. `sessionId` is empty for a draft:
   * nothing exists on that host yet, so there is no shell to offer.
   */
  remote?: {
    desktopId: string;
    sessionId: string;
    /** Whether that host advertised `terminal_v1`. */
    terminalSupported: boolean;
  } | null;
}

/** The `localStorage` key prefix that cannot collide with a local thread id. */
export const REMOTE_SCOPE_PREFIX = "remote:";

export function remoteScopeKey(desktopId: string, sessionId: string): string {
  return `${REMOTE_SCOPE_PREFIX}${desktopId}:${sessionId}`;
}

export function terminalScope(input: TerminalScopeInput): TerminalScope | null {
  if (input.centerMode !== "thread")
    return null;
  if (input.section !== "chat" && input.section !== "workspace")
    return null;
  const remote = input.remote;
  if (remote && remote.sessionId) {
    if (!remote.terminalSupported)
      return null;
    return {
      kind: "remote",
      desktopId: remote.desktopId,
      sessionId: remote.sessionId,
      key: remoteScopeKey(remote.desktopId, remote.sessionId),
    };
  }
  if (!input.threadId)
    return null;
  return { kind: "local", threadId: input.threadId, key: input.threadId };
}

/**
 * A remote conversation that would have a terminal, if only its host served one.
 *
 * `terminalScope` answers "offer the panel"; this answers the narrower question
 * its refusal leaves open: *which* refusal is it. A conversation on a host that
 * does not advertise `terminal_v1` is a different situation from a draft or a
 * view with no conversation at all — the machine is fine, it just serves no
 * shells — and the two must not look alike. The header renders a disabled
 * affordance for this case so the reader sees why the control is missing;
 * silence reads as a bug, and did (a paired machine on a build without the
 * capability looked exactly like a broken window).
 */
export function remoteTerminalUnsupported(input: TerminalScopeInput): boolean {
  if (input.centerMode !== "thread")
    return false;
  if (input.section !== "chat" && input.section !== "workspace")
    return false;
  const remote = input.remote;
  return Boolean(remote && remote.sessionId && !remote.terminalSupported);
}
