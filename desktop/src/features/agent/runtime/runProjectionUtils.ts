import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredRunEvent } from "../../../integrations/storage/threadStore";
import { buildAssistantRunProjection } from "@future-os/thread-projection";
import { getRun, listRunEvents } from "../../../integrations/storage/threadStore";

/** Apply a patch to the single message with `id`, leaving the rest untouched. */
export function patchMessage(
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  id: string,
  patch: Partial<AgentMessage> | ((message: AgentMessage) => Partial<AgentMessage>),
) {
  setMessages(current =>
    current.map(message =>
      message.id === id
        ? { ...message, ...(typeof patch === "function" ? patch(message) : patch) }
        : message,
    ),
  );
}

/**
 * Preserve event segments even when no answer text arrived. The RPC reply is
 * authoritative for missing text, but must not replace reasoning/tool output
 * that was already visible during streaming.
 */
export function deriveRenderFields(
  events: StoredRunEvent[],
  fallbackContent: string,
  /**
   * What the bubble already renders — the live lane's own accumulation, which
   * is the only copy left when the run's log comes back empty (see the final
   * return below). Optional: a caller reading a run it never watched has
   * nothing to preserve.
   */
  previous?: Pick<AgentMessage, "segments" | "outputTokens">,
): { content: string; segments?: MessageSegment[]; outputTokens: number } {
  const projection = buildAssistantRunProjection(events);
  if (projection.content.trim()) {
    return {
      content: projection.content,
      segments: projection.segments,
      outputTokens: projection.outputTokens,
    };
  }
  if (projection.segments.length > 0) {
    return {
      content: fallbackContent,
      segments: fallbackContent.trim()
        ? [...projection.segments, { id: "rpc_fallback_text", kind: "text", text: fallbackContent }]
        : projection.segments,
      outputTokens: projection.outputTokens,
    };
  }
  // An empty projection is an absence of evidence, not evidence of an empty
  // reply. A settled run's raw events are retired into a projection snapshot
  // the moment it completes, so re-reading its log (and a build that cannot
  // expand the snapshot) legitimately returns nothing — even though the bubble
  // streamed a full timeline for minutes. Downgrading to `content` alone drops
  // `segments`, and with them the chronological order the renderer uses: the
  // reply comes back as prose followed by its tool rows, and its token count
  // resets to zero. Keep what the live lane already had.
  return {
    content: fallbackContent,
    ...(previous?.segments?.length ? { segments: previous.segments } : {}),
    // A usage-only projection still carries the run's real count; only fall
    // back to what the bubble had when the log itself was empty.
    outputTokens: projection.outputTokens || previous?.outputTokens || 0,
  };
}

export async function safeListRunEvents(runId: string): Promise<StoredRunEvent[]> {
  try {
    return await listRunEvents(runId);
  }
  catch {
    return [];
  }
}

/**
 * Exact model run time from the persisted run; falls back to wall-clock since
 * the send anchor while the run is still settling. Null when neither is known.
 */
export function runDurationMs(run: StoredRun | null | undefined, fallbackStartMs?: number): number | null {
  if (run?.startedAt && run?.endedAt && run.endedAt >= run.startedAt) {
    return run.endedAt - run.startedAt;
  }
  if (typeof fallbackStartMs === "number") {
    return Math.max(0, Date.now() - fallbackStartMs);
  }
  return null;
}

let clientIdCounter = 0;

export function clientId(prefix: string) {
  clientIdCounter += 1;
  return `${prefix}_${Date.now()}_${clientIdCounter}`;
}

/**
 * A run's current record by id (settle checks) — direct PK lookup, not a
 * full per-thread list.
 */
export async function loadCurrentRun(runId: string) {
  try {
    return await getRun(runId);
  }
  catch {
    return null;
  }
}
