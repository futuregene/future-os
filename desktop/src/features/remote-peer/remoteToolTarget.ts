import { asToolKind, normalizeArgs, targetFromArgs } from "@future-os/thread-projection";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * What a tool call actually did, for a transcript that only received its identity.
 *
 * A lean history page strips a shell call's arguments: the row keeps the call id
 * and the run, and the command is fetched back when it is opened. The desktop
 * declares no lean capability, but a long conversation is still byte-budgeted, so
 * a row can arrive with its identity and nothing else — which is why this asks.
 *
 * The parsing is the *shared projection package's*, not a second copy: mobile
 * derives a row's target from exactly these three functions, and two
 * implementations of "where did this tool act" would drift into disagreeing
 * about which field carries the path.
 */

/** One fetched call's arguments, as the host reports them. */
interface ToolCallArgs {
  arguments?: unknown;
  name?: string;
}

/**
 * A fetched target per `(desktop, session, run, call)`.
 *
 * Cached because a transcript re-renders on every streaming push, and a row that
 * refetched its command each time would turn reading a conversation into a
 * request storm. The value is a *promise* so two rows asking at once make one
 * request, and so a failure is remembered rather than retried on every render.
 */
const targets = new Map<string, Promise<string | null>>();

function cacheKey(desktopId: string, sessionId: string, runId: string, toolCallId: string): string {
  return `${desktopId}::${sessionId}::${runId}::${toolCallId}`;
}

/**
 * The target for one tool call: a shell command, or the path a file tool touched.
 *
 * `null` means the host did not report one, which is not an error — a tool with
 * no target is a tool this UI simply shows by name.
 */
export function fetchRemoteToolTarget(input: {
  desktopId: string;
  sessionId: string;
  runId: string;
  toolCallId: string;
}): Promise<string | null> {
  const key = cacheKey(input.desktopId, input.sessionId, input.runId, input.toolCallId);
  const cached = targets.get(key);
  if (cached)
    return cached;
  const pending = requestRemotePeer<ToolCallArgs>(
    input.desktopId,
    {
      type: "get_tool_call_args",
      sessionId: input.sessionId,
      runId: input.runId,
      toolCallId: input.toolCallId,
    },
    input.sessionId,
  ).then((data) => {
    const name = typeof data?.name === "string" ? data.name : "";
    const args = normalizeArgs(data?.arguments ?? null);
    return targetFromArgs(asToolKind(name), args) ?? null;
  }).catch(() => {
    // A failure is remembered as "no target": the row falls back to its name,
    // and a transcript that retried a failed read per render would be worse than
    // one that shows less.
    return null;
  });
  targets.set(key, pending);
  return pending;
}

/** Drop a conversation's cached targets, so a fresh open re-reads them. */
export function forgetRemoteToolTargets(desktopId: string, sessionId: string): void {
  const prefix = `${desktopId}::${sessionId}::`;
  for (const key of [...targets.keys()]) {
    if (key.startsWith(prefix))
      targets.delete(key);
  }
}
