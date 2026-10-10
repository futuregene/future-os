import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRun } from "../../../integrations/storage/threadStore";
import {
  entriesToTurns,
  matchesSettledRun,
  turnsToMessages,
} from "@future-os/thread-projection";
import {
  getRun,
  getSessionEntriesPage,
  listRuns,
} from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import {
  applyJournalRunOutcomes,
  applyRunMetadata,
  buildStreamingPreview,
  mergeStreamingPreview,
  recoverAbortedTurns,
  recoverFailedRuns,
} from "../threadRunProjection";

export type AgentLoadResult
  = | {
    status: "loaded";
    messages: AgentMessage[];
    hasMore: boolean;
    nextOffset: number;
  }
  | { status: "failed"; error: string };

export async function loadThreadHistory(
  threadId: string,
  activeRunId?: string | null,
  activeRunStartedAt?: number | null,
  before: number | null = null,
): Promise<AgentLoadResult> {
  try {
    const result = await getSessionEntriesPage(threadId, before);
    const entries = result?.entries ?? [];
    if (!entries.length) {
      return {
        status: "loaded",
        messages: [],
        hasMore: result.hasMore,
        nextOffset: result.nextOffset,
      };
    }
    const turns = entriesToTurns(entries as unknown as import("@future-os/thread-projection").SessionEntry[]);
    // The shared projection intentionally leaves run identity off user
    // bubbles. Desktop reconciliation needs both halves of an exchange;
    // take identity from the canonical turn (never guess by text/time).
    const userRuns = new Map(turns.flatMap(node =>
      node.kind === "turn" && node.turn.runId
        ? [[node.turn.user.id, node.turn.runId] as const]
        : [],
    ));
    const messages = applyJournalRunOutcomes(turnsToMessages(turns));
    if (!messages.length) {
      return {
        status: "loaded",
        messages: [],
        hasMore: result.hasMore,
        nextOffset: result.nextOffset,
      };
    }
    // Agent transcript doesn't record a run's GUI-side outcome (failed/cancelled/
    // model) — backfill it from the SQLite `runs` table so a reload keeps the
    // Retry/Continue button, the "stopped" marker, and the model badge.
    const allRuns = await listRuns(threadId).catch(() => [] as StoredRun[]);
    // Do not recover failures or events from unloaded history into this page.
    const firstTime = Date.parse(messages[0]!.createdAt);
    const lastTime
      = before === null
        ? Infinity
        : Date.parse(messages[messages.length - 1]!.createdAt);
    const runs = allRuns.filter((run) => {
      const time = run.startedAt ?? run.createdAt;
      return (
        (!result.hasMore || (run.endedAt ?? run.updatedAt) >= firstTime)
        && time <= lastTime
      );
    });
    const withRunMeta = applyRunMetadata(messages, runs);
    // An aborted exchange has no reply in the Agent transcript — recover the partial
    // text the model streamed (persisted as run events) so it isn't lost.
    const recovered = await recoverAbortedTurns(withRunMeta);
    // A run that failed before any assistant entry was saved (e.g. the model
    // API rejected the first call) leaves no trace in the Agent transcript —
    // rebuild its failure bubble from the run record so the error survives a
    // thread switch instead of silently disappearing.
    const withFailures = recoverFailedRuns(recovered, runs);
    // An in-flight run is folded into the SAME array here: history and live
    // bubble land in one setMessages, so opening an active conversation
    // paints both in a single frame instead of history then bubble. The fold
    // also dedups a mid-run snapshot the agent's save_callback may already
    // have persisted for this exchange (mergeStreamingPreview). Verified
    // against the run row so a settle that raced the reload never resurrects
    // a bubble for a finished run.
    const liveBubble = activeRunId
      ? await getRun(activeRunId)
          .then(run =>
            run && !matchesSettledRun(run.status)
              ? buildStreamingPreview(activeRunId, activeRunStartedAt ?? null)
              : null,
          )
          .catch(() => null)
      : null;
    const finalMessages = liveBubble
      ? mergeStreamingPreview(withFailures, liveBubble)
      : withFailures;
    return {
      status: "loaded",
      messages: finalMessages.map(message =>
        message.role === "user" && userRuns.has(message.id)
          ? { ...message, runId: userRuns.get(message.id) }
          : message,
      ),
      hasMore: result.hasMore,
      nextOffset: result.nextOffset,
    };
  }
  catch (error) {
    return { status: "failed", error: errorMessage(error) };
  }
}
