import type { AgentMessage } from "@future-os/thread-projection";

/** Keep recent conversation snapshots warm across keyed AgentThread remounts. */
const MAX_CACHED_THREADS = 12;

interface ThreadMessageSnapshot {
  agentSessionId: string | null;
  messages: AgentMessage[];
  cursor?: { before: number; hasMore: boolean };
}

const snapshots = new Map<string, ThreadMessageSnapshot>();

export function getThreadMessageSnapshot(
  threadId: string,
  agentSessionId: string | null,
): AgentMessage[] | null {
  const snapshot = snapshots.get(threadId);
  if (!snapshot || snapshot.agentSessionId !== agentSessionId)
    return null;
  // Map insertion order is our LRU order.
  snapshots.delete(threadId);
  snapshots.set(threadId, snapshot);
  return snapshot.messages;
}

export function getThreadHistoryCursor(
  threadId: string,
  agentSessionId: string | null,
) {
  const snapshot = snapshots.get(threadId);
  return snapshot?.agentSessionId === agentSessionId
    ? (snapshot.cursor ?? null)
    : null;
}

export function setThreadMessageSnapshot(
  threadId: string,
  agentSessionId: string | null,
  messages: AgentMessage[],
  cursor?: { before: number; hasMore: boolean },
) {
  snapshots.delete(threadId);
  snapshots.set(threadId, { agentSessionId, messages, cursor });
  while (snapshots.size > MAX_CACHED_THREADS) {
    const oldest = snapshots.keys().next().value;
    if (oldest === undefined)
      break;
    snapshots.delete(oldest);
  }
}

/** Test-only reset; exported to keep the cache module independently testable. */
export function clearThreadMessageSnapshots() {
  snapshots.clear();
}
