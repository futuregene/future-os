import type { AgentMessage } from "@future-os/thread-projection";

type PersistedEntry = Record<string, unknown>;

/** Resolve a UI user bubble to its ordinal among persisted user entries. */
export function persistedUserMessageIndex(
  entries: PersistedEntry[],
  message: Pick<AgentMessage, "id" | "runId">,
): number {
  const runId = typeof message.runId === "string" && message.runId
    ? message.runId
    : null;
  return entries
    .filter(entry => entry.role === "user")
    .findIndex(entry =>
      (runId !== null && entry.runId === runId)
      || (typeof entry.id === "string" && `m_${entry.id}` === message.id),
    );
}
